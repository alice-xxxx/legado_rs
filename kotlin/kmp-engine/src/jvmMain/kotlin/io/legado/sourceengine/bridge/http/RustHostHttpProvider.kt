package io.legado.sourceengine.bridge.http

import io.legado.app.help.http.KmpHttpClient
import io.legado.app.help.http.OkHttpClientProvider
import io.legado.app.help.http.OkHttpClientProviders
import io.legado.app.help.http.OkHttpProxyClientProvider
import io.legado.app.help.http.OkHttpProxyClientProviders
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.int
import kotlinx.serialization.json.put
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Headers
import okhttp3.Interceptor
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import java.io.IOException
import java.util.Base64
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.TimeUnit
import io.legado.app.help.http.cookieJarHeader
import io.legado.sourceengine.bridge.HostHeader
import io.legado.sourceengine.bridge.HostHttpRequest
import io.legado.sourceengine.bridge.HostHttpResponse
import io.legado.sourceengine.bridge.SourceEngineHost
import io.legado.sourceengine.bridge.SourceEngineHostRegistry
import kotlinx.coroutines.runBlocking

/**
 * 桌面无界面宿主使用的 HTTP provider。书源流程仍调用项目熟悉的 OkHttp API，
 * 但每个请求（含 JS ajax、jsLib 下载）都会通过 SourceEngineHost 转交 Rust reqwest 真正联网。
 * 这样复用规则层的 URL、Header、Cookie 与响应解析语义，同时把 socket/代理等宿主网络 I/O 留在 Rust。
 *
 * 来源标记：这是为独立 Rust/JNI 宿主新写的适配层，不是从某个平台网络实现复制来的。
 * 它接入仓库原 data 模块的 OkHttpClientProvider、OkHttpProxyClientProvider 和 KmpHttpTypes
 * 接口；规则请求的构造/解析仍由原书源流程完成，实际 socket 请求通过宿主桥交给 Rust。
 * 对应接口定义和原始公共请求代码见本模块 README 的来源表。
 */
object RustHostHttpProvider : OkHttpClientProvider, OkHttpProxyClientProvider {
    // 复用 shared client 的请求/响应策略（header、Cookie 回调、解压），但在应用拦截器层
    // 返回 Rust 响应；因此不会继续走到 OkHttp 的 socket 层。
    private val delegate by lazy { io.legado.app.help.http.okHttpClient }
    private val requestSequence = AtomicLong()
    private val proxiedClients = ConcurrentHashMap<String, KmpHttpClient>()
    @Volatile private var nativeExchange: ((String, String) -> String)? = null

    override val okHttpClient: KmpHttpClient by lazy {
        bridge(delegate, null)
    }

    override fun getProxyClient(proxy: String?): KmpHttpClient {
        if (proxy.isNullOrBlank()) return okHttpClient
        return proxiedClients.getOrPut(proxy) {
            bridge(delegate.newBuilder().build(), proxy)
        }
    }

    private fun register(): RustHostHttpProvider {
        OkHttpClientProviders.register(this)
        OkHttpProxyClientProviders.impl = this
        return this
    }

    /**
     * 桌面嵌入式 JVM 的同步 JNI 宿主回调。规则侧保留 OkHttp 请求构造和响应语义，
     * HTTP 与持久化 RPC 都经同步 JNI 回调进入创建 JVM 的 Rust 进程。
     */
    fun installNative(): RustHostHttpProvider {
        nativeExchange = { type, event ->
            when (type) {
                "httpRequest" -> nativeHttpRequest(event)
                "storageRequest" -> nativeStorageRequest(event)
                else -> error("Unsupported native source-engine RPC: $type")
            }
        }
        return register()
    }

    fun close() {
        (listOfNotNull(runCatching { okHttpClient }.getOrNull()) + proxiedClients.values)
            .distinct()
            .forEach { client ->
                runCatching { client.dispatcher.executorService.shutdown() }
                runCatching { client.connectionPool.evictAll() }
            }
    }

    private fun bridge(client: KmpHttpClient, proxy: String?): KmpHttpClient {
        val builder = client.newBuilder()
        // HttpHelper 会在最终桥接前读取并移除内部 CookieJar 标记；把“是否启用 CookieJar”
        // 转成请求元数据发给 Rust，避免内部控制 header 泄漏到书源站点。
        builder.interceptors().add(0, Interceptor { chain ->
            val request = chain.request()
            val hasCookieJar = request.header(cookieJarHeader) != null
            chain.proceed(
                request.newBuilder()
                    .tag(BridgeRequestMetadata::class.java, BridgeRequestMetadata(hasCookieJar))
                    .build()
            )
        })
        builder.addInterceptor(Interceptor { chain -> executeThroughRust(chain, proxy) })
        return builder.build()
    }

    private fun executeThroughRust(chain: Interceptor.Chain, proxy: String?): Response {
        val request = chain.request()
        val bodyBuffer = Buffer()
        request.body?.writeTo(bodyBuffer)

        // 没有活动 Rust Host 就明确失败，避免 source request 静默回退到 OkHttp socket。
        val host = request.tag(SourceEngineHost::class.java)
            ?: SourceEngineHostRegistry.current()
            ?: throw IOException("No active Rust source-engine host")
        val hostHeaders = request.headers.mapNotNull { (name, value) ->
            if (name.lowercase() in FORBIDDEN_HEADERS) null else HostHeader(name, value)
        }.toMutableList()
        // OkHttp 将媒体类型保存在 RequestBody；补回请求头后 Rust 才能保留表单/JSON 编码语义。
        if (hostHeaders.none { it.name.equals("Content-Type", ignoreCase = true) }) {
            request.body?.contentType()?.let { hostHeaders += HostHeader("Content-Type", it.toString()) }
        }
        val hostRequest = HostHttpRequest(
            url = request.url.toString(),
            method = request.method,
            headers = hostHeaders,
            body = request.body?.let { bodyBuffer.readByteArray() },
            connectTimeoutMs = chain.connectTimeoutMillis().takeIf { it > 0 }?.toLong(),
            readTimeoutMs = chain.readTimeoutMillis().takeIf { it > 0 }?.toLong(),
            callTimeoutMs = chain.call().timeout().timeoutNanos().takeIf { it > 0L }?.let {
                TimeUnit.NANOSECONDS.toMillis(it).coerceAtLeast(1L)
            },
            maxRedirects = 20,
            useCookieJar = request.tag(BridgeRequestMetadata::class.java)?.useCookieJar == true,
            proxy = proxy,
            acceptInvalidCerts = true,
        )
        val response = runBlocking { host.executeHttp(hostRequest) }
        val responseHeaders = Headers.Builder().apply {
            response.headers.forEach { add(it.name, it.value) }
        }.build()
        val responseBody = if (response.status == 204 || response.status == 304) {
            ByteArray(0).toResponseBody(null)
        } else {
            response.body.toResponseBody(responseHeaders["Content-Type"]?.toMediaTypeOrNull())
        }
        return Response.Builder()
            .request(request.newBuilder().url(response.finalUrl).build())
            .protocol(Protocol.HTTP_1_1)
            .code(response.status)
            .message(response.reason)
            .headers(responseHeaders)
            .body(responseBody)
            .build()
    }

    /** 把 KMP 宿主请求编码为 Rust HTTP RPC，再通过当前 JNI 回调交给桌面 Rust Host。 */
    suspend fun executeHostHttp(request: HostHttpRequest): HostHttpResponse {
        val id = requestSequence.incrementAndGet().toString()
        val event = buildJsonObject {
            put("type", "httpRequest")
            put("id", id)
            put("request", buildJsonObject {
                put("url", request.url)
                put("method", request.method)
                put("headers", buildJsonArray {
                    request.headers.forEach { header ->
                        add(buildJsonObject {
                            put("name", header.name)
                            put("value", header.value)
                        })
                    }
                })
                request.body?.let { put("bodyBase64", Base64.getEncoder().encodeToString(it)) }
                if (request.multipart.isNotEmpty()) {
                    put("multipart", buildJsonArray {
                        request.multipart.forEach { part ->
                            add(buildJsonObject {
                                put("name", part.name)
                                part.text?.let { put("text", it) }
                                part.fileName?.let { put("fileName", it) }
                                part.contentType?.let { put("contentType", it) }
                                part.bytes?.let { put("bytesBase64", Base64.getEncoder().encodeToString(it)) }
                            })
                        }
                    })
                }
                request.connectTimeoutMs?.let { put("connectTimeoutMs", it) }
                request.readTimeoutMs?.let { put("readTimeoutMs", it) }
                request.callTimeoutMs?.let { put("callTimeoutMs", it) }
                request.maxRedirects?.let { put("maxRedirects", it) }
                put("useCookieJar", request.useCookieJar)
                request.proxy?.let { put("proxy", it) }
                put("acceptInvalidCerts", request.acceptInvalidCerts)
            })
        }
        val reply = exchange("httpRequest", event)
        if (reply["ok"]?.jsonPrimitive?.contentOrNull != "true") {
            throw IOException(reply["error"]?.jsonPrimitive?.contentOrNull ?: "Rust HTTP request failed")
        }
        val response = reply["response"]?.jsonObject
            ?: throw IOException("Rust HTTP bridge returned no response")
        val headers = response["headers"]?.jsonArray.orEmpty().map { header ->
            val item = header.jsonObject
            HostHeader(
                item.getValue("name").jsonPrimitive.contentOrNull.orEmpty(),
                item.getValue("value").jsonPrimitive.contentOrNull.orEmpty(),
            )
        }
        return HostHttpResponse(
            requestedUrl = response["requestedUrl"]?.jsonPrimitive?.contentOrNull ?: request.url,
            finalUrl = response["finalUrl"]?.jsonPrimitive?.contentOrNull
                ?: throw IOException("Rust HTTP response has no final URL"),
            status = response["status"]?.jsonPrimitive?.int
                ?: throw IOException("Rust HTTP response has no status"),
            reason = response["reason"]?.jsonPrimitive?.contentOrNull.orEmpty(),
            headers = headers,
            body = Base64.getDecoder().decode(response["bodyBase64"]?.jsonPrimitive?.contentOrNull.orEmpty()),
        )
    }

    /**
     * 同步存储 RPC：shared provider 暴露的是同步接口，由 JNI 回调进入 Rust Host；
     * namespace 白名单和落盘路径由 Rust 管理，Kotlin 不自行拼宿主文件路径。
     */
    fun storage(
        namespace: String,
        operation: String,
        key: String? = null,
        value: String? = null,
        ttlSeconds: Int? = null,
        keyPrefixes: List<String>? = null,
        contains: String? = null,
    ): String? {
        check(nativeExchange != null) {
            "Rust source-engine native host is not installed"
        }
        val id = requestSequence.incrementAndGet().toString()
        val request = buildJsonObject {
            put("type", "storageRequest")
            put("id", id)
            put("namespace", namespace)
            put("operation", operation)
            key?.let { put("key", it) }
            if (operation == "put") put("value", value?.let(::JsonPrimitive) ?: JsonNull)
            ttlSeconds?.let { put("ttlSeconds", it) }
            keyPrefixes?.let { prefixes ->
                put("keyPrefixes", buildJsonArray { prefixes.forEach { add(JsonPrimitive(it)) } })
            }
            contains?.let { put("contains", it) }
        }
        val reply = exchange("storageRequest", request)
        if (reply["ok"]?.jsonPrimitive?.contentOrNull != "true") {
            throw IOException(reply["error"]?.jsonPrimitive?.contentOrNull ?: "Rust storage request failed")
        }
        return reply["value"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.contentOrNull
    }

    private fun exchange(type: String, event: JsonObject): JsonObject {
        val callback = nativeExchange
            ?: throw IOException("Rust source-engine native host is not installed")
        val response = callback(type, event.toString())
        return try {
            kotlinx.serialization.json.Json.parseToJsonElement(response).jsonObject
        } catch (error: Exception) {
            throw IOException("Rust native $type returned invalid JSON: ${error.message}", error)
        }
    }

    private val FORBIDDEN_HEADERS = setOf(
        "connection",
        "content-length",
        "host",
        "keep-alive",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "transfer-encoding",
        "upgrade",
    )
}

private data class BridgeRequestMetadata(val useCookieJar: Boolean)
