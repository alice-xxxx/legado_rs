package io.legado.sourceengine.bridge

import io.legado.app.help.http.DecompressInterceptor
import io.legado.app.help.http.KmpHttpClient
import io.legado.app.help.http.OkHttpClientProvider
import io.legado.app.help.http.OkHttpClientProviders
import io.legado.app.help.http.OkHttpProxyClientProvider
import io.legado.app.help.http.OkHttpProxyClientProviders
import io.legado.app.help.http.cookieJarHeader
import io.legado.app.utils.NetworkUtils
import kotlinx.coroutines.runBlocking
import okhttp3.Headers
import okhttp3.Interceptor
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okio.Buffer
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/**
 * 独立 Android 目标的 HTTP provider。书源请求必须携带当前 Rust host；缺少 host 时直接报错，
 * 避免解析器因漏传 request tag 而静默改走 Android socket。
 */
object AndroidRustSourceHttpProvider : OkHttpClientProvider, OkHttpProxyClientProvider {
    private val proxyClients = ConcurrentHashMap<String, KmpHttpClient>()

    override val okHttpClient: KmpHttpClient by lazy { createClient(proxy = null) }

    fun install() {
        OkHttpClientProviders.register(this)
        OkHttpProxyClientProviders.impl = this
    }

    override fun getProxyClient(proxy: String?): KmpHttpClient =
        proxy?.takeIf { it.isNotBlank() }?.let { proxyClients.getOrPut(it) { createClient(it) } }
            ?: okHttpClient

    private fun createClient(proxy: String?): KmpHttpClient = OkHttpClient.Builder()
        // 压缩仍由原解析器拦截器处理；实际连接和 Cookie Jar 都归 Rust 管理。
        .addInterceptor(DecompressInterceptor)
        .addInterceptor(RustHttpInterceptor(proxy))
        .build()
}

private class RustHttpInterceptor(private val proxy: String?) : Interceptor {
    override fun intercept(chain: Interceptor.Chain): Response {
        val request = chain.request()
        val host = request.tag(SourceEngineHost::class.java)
            ?: SourceEngineHostRegistry.current()
            ?: throw IOException("Source request has no active Rust host")

        val bodyBuffer = Buffer()
        request.body?.writeTo(bodyBuffer)
        val headers = request.headers.mapNotNull { (name, value) ->
            if (name.lowercase() in RUST_OWNED_HEADERS || name.equals(cookieJarHeader, ignoreCase = true)) {
                null
            } else {
                HostHeader(name, value)
            }
        }.toMutableList()
        if (headers.none { it.name.equals("Content-Type", ignoreCase = true) }) {
            request.body?.contentType()?.let { headers += HostHeader("Content-Type", it.toString()) }
        }
        val useCookieJar = request.header(cookieJarHeader) != null

        val rustResponse = runBlocking {
            host.executeHttp(
                HostHttpRequest(
                    url = request.url.toString(),
                    method = request.method,
                    headers = headers,
                    body = request.body?.let { bodyBuffer.readByteArray() },
                    connectTimeoutMs = chain.connectTimeoutMillis().takeIf { it > 0 }?.toLong(),
                    readTimeoutMs = chain.readTimeoutMillis().takeIf { it > 0 }?.toLong(),
                    callTimeoutMs = chain.call().timeout().timeoutNanos().takeIf { it > 0L }?.let {
                        TimeUnit.NANOSECONDS.toMillis(it).coerceAtLeast(1L)
                    },
                    maxRedirects = 20,
                    useCookieJar = useCookieJar,
                    cookieScope = if (useCookieJar) {
                        NetworkUtils.getSubDomain(request.url.toString())
                    } else {
                        null
                    },
                    proxy = proxy,
                    acceptInvalidCerts = true,
                ),
            )
        }
        val responseHeaders = Headers.Builder().apply {
            rustResponse.headers.forEach { add(it.name, it.value) }
        }.build()
        val responseBody = if (rustResponse.status == 204 || rustResponse.status == 304) {
            ByteArray(0).toResponseBody(null)
        } else {
            rustResponse.body.toResponseBody(responseHeaders["Content-Type"]?.toMediaTypeOrNull())
        }
        var priorResponse: Response? = null
        rustResponse.redirects.forEach { redirect ->
            val redirectBuilder = Response.Builder()
                .request(Request.Builder().url(redirect.fromUrl.toHttpUrl()).build())
                .protocol(Protocol.HTTP_1_1)
                .code(redirect.status)
                // Only the actual status and destination are available from reqwest.
                .message("")
                .header("Location", redirect.toUrl)
            priorResponse?.let(redirectBuilder::priorResponse)
            priorResponse = redirectBuilder.build()
        }
        return Response.Builder()
            .request(request.newBuilder().url(rustResponse.finalUrl.toHttpUrl()).build())
            .protocol(Protocol.HTTP_1_1)
            .code(rustResponse.status)
            .message(rustResponse.reason)
            .headers(responseHeaders)
            .body(responseBody)
            .apply { priorResponse?.let { this.priorResponse(it) } }
            .build()
    }
}

private val RUST_OWNED_HEADERS = setOf(
    "connection", "content-length", "host", "keep-alive", "proxy-authorization",
    "proxy-connection", "te", "transfer-encoding", "upgrade",
)
