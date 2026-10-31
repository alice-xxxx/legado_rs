// iOS HTTP 数据模型和宿主请求转换，不包含另一套网络客户端。
@file:OptIn(ExperimentalEncodingApi::class)

package io.legado.app.help.http

import io.legado.app.constant.AppConst
import io.legado.app.help.UserAgentProviders
import io.legado.sourceengine.host.HostHeader
import io.legado.sourceengine.host.HostHttpRedirect
import io.legado.sourceengine.host.HostHttpRequest
import io.legado.sourceengine.host.SourceEngineHost
import io.legado.sourceengine.host.SourceEngineHostRegistry
import io.legado.app.utils.Closeable
import io.legado.app.utils.InputStream
import io.legado.app.utils.NetworkUtils
import io.legado.app.utils.toInputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import okio.IOException
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi
import kotlin.reflect.KClass
import kotlin.time.Duration
import okio.Buffer
import okio.GzipSource
import okio.Inflater
import okio.InflaterSource
import okio.buffer
import okio.use
actual interface KmpInterceptor {
    actual fun intercept(chain: KmpInterceptorChain): KmpResponse
}

actual interface KmpInterceptorChain {
    actual fun request(): KmpRequest
    actual fun proceed(request: KmpRequest): KmpResponse
}

actual class KmpHttpClient {
    internal var readTimeoutMillis=15_000L
    internal var callTimeoutMillis=15_000L
    internal var proxyUrl:String?=null

    constructor()
    internal constructor(builder:KmpHttpClientBuilder){
        readTimeoutMillis=builder.readTimeoutMillis.takeIf{it>0}?:15_000L
        callTimeoutMillis=builder.callTimeoutMillis.takeIf{it>0}?:15_000L
        proxyUrl=builder.proxyUrl
    }
    actual fun newCall(request:KmpRequest):KmpCall=NativeKmpCall(this,request)
    actual fun newBuilder()=KmpHttpClientBuilder().also{
        it.readTimeoutMillis=readTimeoutMillis;it.callTimeoutMillis=callTimeoutMillis
        it.proxyUrl=proxyUrl
    }
}
actual class KmpHttpClientBuilder actual constructor(){
    internal var readTimeoutMillis=0L
    internal var callTimeoutMillis=0L
    internal var proxyUrl:String?=null
    actual fun readTimeout(duration:Duration)=apply{readTimeoutMillis=duration.inWholeMilliseconds}
    actual fun callTimeout(duration:Duration)=apply{callTimeoutMillis=duration.inWholeMilliseconds}
    internal fun proxy(url:String)=apply{proxyUrl=url}
    actual fun build()=KmpHttpClient(this)
}

actual class KmpRequest {
    internal var urlStr: String = "http://localhost/"
        private set
    internal var method: String = "GET"
        private set
    internal var headers: List<Pair<String, String>> = emptyList()
        private set
    internal var body: KmpRequestBody? = null
        private set
    internal var tags: Map<KClass<*>, Any> = emptyMap()
        private set

    constructor()

    internal constructor(
        urlStr: String,
        method: String,
        headers: List<Pair<String, String>>,
        body: KmpRequestBody?,
        tags: Map<KClass<*>, Any> = emptyMap(),
    ) {
        this.urlStr = urlStr
        this.method = method
        this.headers = headers
        this.body = body
        this.tags = tags
    }

    actual fun newBuilder(): KmpRequestBuilder {
        return KmpRequestBuilder().also { b ->
            b.urlStr = urlStr
            b.method = method
            b.headers.addAll(headers)
            b.body = body
            b.tags.putAll(tags)
        }
    }

    actual fun header(name: String): String? =
        headers.firstOrNull { it.first.equals(name, ignoreCase = true) }?.second

    actual val url: KmpHttpUrl
        get() = KmpHttpUrl(urlStr)
}

actual class KmpRequestBuilder actual constructor() {
    internal var urlStr: String = "http://localhost/"
    internal var method: String = "GET"
    internal val headers: MutableList<Pair<String, String>> = mutableListOf()
    internal var body: KmpRequestBody? = null
    internal val tags: MutableMap<KClass<*>, Any> = LinkedHashMap()

    actual fun url(url: String): KmpRequestBuilder {
        urlStr = url
        return this
    }

    actual fun url(url: KmpHttpUrl): KmpRequestBuilder {
        urlStr = url.urlStr ?: "http://localhost/"
        return this
    }

    actual fun addHeader(name: String, value: String): KmpRequestBuilder {
        headers.add(name to value)
        return this
    }

    actual fun header(name: String, value: String): KmpRequestBuilder {
        headers.removeAll { it.first.equals(name, ignoreCase = true) }
        headers.add(name to value)
        return this
    }

    actual fun removeHeader(name: String): KmpRequestBuilder {
        headers.removeAll { it.first.equals(name, ignoreCase = true) }
        return this
    }

    actual fun get(): KmpRequestBuilder {
        method = "GET"
        body = null
        return this
    }

    actual fun post(body: KmpRequestBody): KmpRequestBuilder {
        method = "POST"
        this.body = body
        return this
    }

    actual fun method(method: String, body: KmpRequestBody?): KmpRequestBuilder {
        this.method = method.uppercase()
        this.body = body
        return this
    }

    actual fun build(): KmpRequest {
        return KmpRequest(urlStr, method, headers.toList(), body, tags.toMap())
    }
}

internal actual fun KmpRequestBuilder.addNonAsciiRequestHeader(
    name: String,
    value: String,
): KmpRequestBuilder = addHeader(name, value)
actual class KmpResponse : Closeable {
    internal var codeVal: Int = 200
        private set
    internal var messageVal: String = "OK"
        private set
    internal var headersVal: Map<String, List<String>> = emptyMap()
        private set
    internal var bodyBytes: ByteArray? = null
        private set
    internal var contentTypeStr: String? = null
        private set
    internal var requestVal: KmpRequest = KmpRequest()
        private set
    internal var priorResponseVal: KmpResponse? = null
        private set
    internal var redirectsVal: List<HostHttpRedirect> = emptyList()

    constructor()
    internal constructor(
        code: Int,
        message: String,
        headers: Map<String, List<String>>,
        body: ByteArray?,
        contentType: String?,
        request: KmpRequest,
        redirects: List<HostHttpRedirect> = emptyList(),
    ) {
        this.codeVal = code
        this.messageVal = message
        this.headersVal = headers
        this.bodyBytes = body
        this.contentTypeStr = contentType
        this.requestVal = request
        this.redirectsVal = redirects.toList()
        this.priorResponseVal = buildPriorResponseChain(request, redirectsVal)
    }

    private fun buildPriorResponseChain(
        request: KmpRequest,
        redirects: List<HostHttpRedirect>,
    ): KmpResponse? {
        var previous: KmpResponse? = null
        redirects.forEachIndexed { index, redirect ->
            val prior = KmpResponse(
                code = redirect.status,
                message = "",
                headers = mapOf("Location" to listOf(redirect.toUrl)),
                body = null,
                contentType = null,
                request = KmpRequest(
                    redirect.fromUrl,
                    request.method,
                    request.headers,
                    request.body,
                    request.tags,
                ),
            )
            prior.redirectsVal = redirects.take(index)
            prior.priorResponseVal = previous
            previous = prior
        }
        return previous
    }

    actual val code: Int get() = codeVal
    actual val message: String get() = messageVal
    actual val body: KmpResponseBody
        get() = NativeKmpResponseBody(bodyBytes ?: ByteArray(0), contentTypeStr)
    actual val isSuccessful: Boolean get() = codeVal in 200..299
    actual val request: KmpRequest get() = requestVal
    actual val networkResponse: KmpResponse? get() = null
    actual val priorResponse: KmpResponse? get() = priorResponseVal
    actual val isRedirect: Boolean get() = codeVal in 300..399

    actual fun headers(): KmpHeaders = KmpHeaders(headersVal)

    actual fun headers(name: String): List<String> =
        headersVal.entries.firstOrNull { it.key.equals(name, ignoreCase = true) }?.value
            ?: emptyList()

    actual fun newBuilder(): KmpResponseBuilder {
        return KmpResponseBuilder().also { b ->
            b.codeVal = codeVal
            b.messageVal = messageVal
            b.headersVal.putAll(headersVal)
            b.bodyBytes = bodyBytes
            b.contentTypeStr = contentTypeStr
            b.requestVal = requestVal
            b.redirectsVal = redirectsVal.toList()
        }
    }

    actual override fun close() {
    }
}
actual fun KmpResponse.header(name: String, defaultValue: String?): String? {
    return headersVal.entries.firstOrNull { it.key.equals(name, ignoreCase = true) }?.value?.firstOrNull()
        ?: defaultValue
}

actual class KmpResponseBuilder actual constructor() {
    internal var codeVal: Int = 200
    internal var messageVal: String = "OK"
    internal val headersVal: MutableMap<String, List<String>> = LinkedHashMap()
    internal var bodyBytes: ByteArray? = null
    internal var contentTypeStr: String? = null
    internal var requestVal: KmpRequest = KmpRequest()
    internal var redirectsVal: List<HostHttpRedirect> = emptyList()

    actual fun code(code: Int): KmpResponseBuilder {
        codeVal = code
        return this
    }

    actual fun message(message: String): KmpResponseBuilder {
        messageVal = message
        return this
    }

    actual fun protocol(protocol: KmpProtocol): KmpResponseBuilder {
        return this
    }

    actual fun request(request: KmpRequest): KmpResponseBuilder {
        requestVal = request
        return this
    }

    actual fun removeHeader(name: String): KmpResponseBuilder {
        headersVal.keys.removeAll { it.equals(name, ignoreCase = true) }
        return this
    }

    actual fun body(body: KmpResponseBody): KmpResponseBuilder {
        val nativeBody = body as? NativeKmpResponseBody
        bodyBytes = nativeBody?.bytesValue
        contentTypeStr = nativeBody?.contentTypeValue
        return this
    }

    actual fun build(): KmpResponse {
        return KmpResponse(
            codeVal,
            messageVal,
            headersVal,
            bodyBytes,
            contentTypeStr,
            requestVal,
            redirectsVal,
        )
    }
}

actual abstract class KmpResponseBody : Closeable {
    actual fun bytes(): ByteArray = (this as NativeKmpResponseBody).bytesValue
    actual fun byteStream(): InputStream = (this as NativeKmpResponseBody).bytesValue.toInputStream()
    actual abstract fun contentType(): KmpMediaType?
    actual fun string(): String = bytes().decodeToString()
    actual override fun close() {
    }

    actual companion object {
        actual val EMPTY: KmpResponseBody
            get() = NativeKmpResponseBody(ByteArray(0), null)
    }
}

internal class NativeKmpResponseBody(
    internal val bytesValue: ByteArray,
    internal val contentTypeValue: String?
) : KmpResponseBody() {
    override fun contentType(): KmpMediaType? = contentTypeValue?.let { NativeKmpMediaType(it) }
}
internal class NativeKmpCall(
    private val client: KmpHttpClient,
    private val request: KmpRequest
) : KmpCall {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var job: Job? = null

    override fun enqueue(responseCallback: KmpCallback) {
        job = scope.launch {
            try {
                val response = executeHost()
                responseCallback.onResponse(this@NativeKmpCall, response)
            } catch (e: Throwable) {
                val ioException = if (e is okio.IOException) e else okio.IOException(e.message, e)
                runCatching { responseCallback.onFailure(this@NativeKmpCall, ioException) }
            }
        }
    }

    override fun cancel() {
        job?.cancel()
        scope.cancel()
    }

    override fun execute(): KmpResponse = runBlocking {
        executeHost()
    }

    private suspend fun executeHost(): KmpResponse {
        val sourceHost = request.tagKmp(SourceEngineHost::class) ?: SourceEngineHostRegistry.current()
        return executeRustHost(sourceHost ?: throw IOException("iOS source request has no active Rust host"))
    }

    private suspend fun executeRustHost(host: SourceEngineHost): KmpResponse {
        val useCookieJar = request.header(cookieJarHeader) != null
        val prepared = request.prepareForSend(bridgeCookieJar = false)
        val body = (prepared.body as? NativeKmpRequestBody)?.bytes
        val hostHeaders = prepared.headers.mapNotNull { (name, value) ->
            if (name.lowercase() in RUST_OWNED_HEADERS || name.equals(cookieJarHeader, ignoreCase = true)) {
                null
            } else {
                HostHeader(name, value)
            }
        }.toMutableList()
        if (hostHeaders.none { it.name.equals("Content-Type", ignoreCase = true) }) {
            (prepared.body as? NativeKmpRequestBody)?.contentType?.let {
                hostHeaders += HostHeader("Content-Type", it.toString())
            }
        }

        val response = host.executeHttp(
            HostHttpRequest(
                url = prepared.urlStr,
                method = prepared.method,
                headers = hostHeaders,
                body = body,
                connectTimeoutMs = DEFAULT_TIMEOUT_MS,
                readTimeoutMs = client.readTimeoutMillis.takeIf { it > 0L },
                callTimeoutMs = client.callTimeoutMillis.takeIf { it > 0L },
                maxRedirects = 20,
                useCookieJar = useCookieJar,
                cookieScope = if (useCookieJar) NetworkUtils.getSubDomain(prepared.urlStr) else null,
                proxy = client.proxyUrl,
                acceptInvalidCerts = true,
            ),
        )
        val groupedHeaders = response.headers.groupBy { it.name }
            .mapValues { (_, values) -> values.map { it.value } }
        val contentEncoding = groupedHeaders.entries
            .firstOrNull { it.key.equals("Content-Encoding", ignoreCase = true) }
            ?.value?.firstOrNull()
        val finalRequest = if (response.finalUrl != prepared.urlStr) {
            KmpRequest(response.finalUrl, prepared.method, prepared.headers, prepared.body, prepared.tags)
        } else {
            prepared
        }
        return KmpResponse(
            code = response.status,
            message = response.reason,
            headers = groupedHeaders,
            body = decompressResponseBody(response.body, contentEncoding),
            contentType = groupedHeaders.entries
                .firstOrNull { it.key.equals("Content-Type", ignoreCase = true) }
                ?.value?.firstOrNull(),
            request = finalRequest,
            redirects = response.redirects,
        )
    }

}

actual interface KmpCall {
    actual fun enqueue(responseCallback: KmpCallback)
    actual fun cancel()
    actual fun execute(): KmpResponse
}

actual interface KmpCallback {
    actual fun onFailure(call: KmpCall, e: okio.IOException)
    actual fun onResponse(call: KmpCall, response: KmpResponse)
}
actual class KmpFormBodyBuilder actual constructor() {
    private val entries: MutableList<Pair<String, String>> = mutableListOf()

    actual fun add(name: String, value: String): KmpFormBodyBuilder {
        entries.add(name to value)
        return this
    }

    actual fun addEncoded(name: String, value: String): KmpFormBodyBuilder {
        entries.add(name to value)
        return this
    }

    internal fun buildFormBody(): KmpRequestBody {
        val sb = StringBuilder()
        entries.forEachIndexed { idx, (k, v) ->
            if (idx > 0) sb.append('&')
            sb.append(urlEncodeForm(k)).append('=').append(urlEncodeForm(v))
        }
        return NativeKmpRequestBody(
            sb.toString().encodeToByteArray(),
            NativeKmpMediaType("application/x-www-form-urlencoded")
        )
    }
}

actual fun KmpFormBodyBuilder.buildKmpRequestBody(): KmpRequestBody = this.buildFormBody()

actual class KmpHttpUrl {
    internal var urlStr: String? = null
        private set

    constructor()

    internal constructor(urlStr: String) {
        this.urlStr = urlStr
    }

    actual fun newBuilder(): KmpHttpUrlBuilder {
        return KmpHttpUrlBuilder(urlStr ?: "http://localhost/")
    }

    override fun toString(): String = urlStr ?: ""
}

actual class KmpHttpUrlBuilder {
    private val baseUrl: String
    private var encodedQueryStr: String? = null
    private val queryParams: MutableList<Pair<String, String>> = mutableListOf()

    constructor(urlStr: String = "http://localhost/") {
        baseUrl = urlStr.substringBefore('?')
        urlStr.substringAfter('?', "").ifNotEmpty { s ->
            s.split('&').forEach { pair ->
                val k = pair.substringBefore('=')
                val v = pair.substringAfter('=', "")
                queryParams.add(k to v)
            }
        }
    }

    actual fun encodedQuery(encodedQuery: String?): KmpHttpUrlBuilder {
        encodedQueryStr = encodedQuery
        return this
    }

    actual fun addQueryParameter(name: String, value: String?): KmpHttpUrlBuilder {
        queryParams.add(name to (value ?: ""))
        return this
    }

    actual fun addEncodedQueryParameter(encodedName: String, encodedValue: String?): KmpHttpUrlBuilder {
        queryParams.add(encodedName to (encodedValue ?: ""))
        return this
    }

    actual fun build(): KmpHttpUrl {
        val sb = StringBuilder(baseUrl)
        if (encodedQueryStr != null) {
            sb.append('?').append(encodedQueryStr)
        } else if (queryParams.isNotEmpty()) {
            sb.append('?')
            queryParams.forEachIndexed { idx, (k, v) ->
                if (idx > 0) sb.append('&')
                sb.append(urlEncodeQuery(k)).append('=').append(urlEncodeQuery(v))
            }
        }
        return KmpHttpUrl(sb.toString())
    }
}

actual open class KmpMediaType {
    internal var value: String = ""
        private set

    constructor()

    internal constructor(value: String) {
        this.value = value
    }

    override fun toString(): String = value
}

actual abstract class KmpRequestBody

internal class NativeKmpRequestBody(
    internal val bytes: ByteArray,
    internal val contentType: KmpMediaType?
) : KmpRequestBody()

actual class KmpHeaders {
    private var map: Map<String, List<String>> = emptyMap()

    constructor()

    constructor(map: Map<String, List<String>>) {
        this.map = map
    }

    actual fun toMultimap(): Map<String, List<String>> = map
}

actual enum class KmpProtocol {
    HTTP_1_0, HTTP_1_1, SPDY_3, HTTP_2, H2_PRIOR_KNOWLEDGE, QUIC
}
actual fun String.toKmpHttpUrl(): KmpHttpUrl = KmpHttpUrl(this)

actual fun String.toKmpMediaType(): KmpMediaType = NativeKmpMediaType(this)

actual fun String.toKmpRequestBody(contentType: KmpMediaType?): KmpRequestBody =
    NativeKmpRequestBody(this.encodeToByteArray(), contentType)

actual fun ByteArray.toKmpRequestBody(contentType: KmpMediaType?): KmpRequestBody =
    NativeKmpRequestBody(this, contentType)

actual fun <T : Any> KmpRequestBuilder.tagKmp(tagClass: KClass<T>, tag: T?): KmpRequestBuilder {
    if (tag == null) tags.remove(tagClass) else tags[tagClass] = tag
    return this
}

@Suppress("UNCHECKED_CAST")
actual fun <T : Any> KmpRequest.tagKmp(tagClass: KClass<T>): T? = tags[tagClass] as? T

actual fun Any?.asKmpResponseBody(contentType: KmpMediaType?, length: Long): KmpResponseBody {
    val bytes = (this as? ByteArray) ?: ByteArray(0)
    return NativeKmpResponseBody(bytes, contentType?.toString())
}
internal class NativeKmpMediaType(value: String) : KmpMediaType(value)

private fun KmpRequest.prepareForSend(bridgeCookieJar: Boolean = true): KmpRequest {
    val builder = newBuilder()
    val ua = header(AppConst.UA_NAME)
    if (ua == null) {
        builder.addHeader(AppConst.UA_NAME, UserAgentProviders.get())
    } else if (ua == "null") {
        builder.removeHeader(AppConst.UA_NAME)
    }
    builder.addHeader("Keep-Alive", "300")
    builder.addHeader("Connection", "Keep-Alive")
    builder.addHeader("Cache-Control", "no-cache")
    if (header("Accept-Encoding") == null && header("Range") == null) {
        builder.addHeader("Accept-Encoding", "gzip, deflate")
    }

    if (header(cookieJarHeader) != null) {
        builder.removeHeader(cookieJarHeader)
        if (bridgeCookieJar) {
            return CookieJarBridgeHolder.get()?.loadRequest(builder.build()) ?: builder.build()
        }
        return builder.build()
    }
    return builder.build()
}

private fun decompressResponseBody(bytes: ByteArray, contentEncoding: String?): ByteArray {
    return when (contentEncoding?.lowercase()) {
        "gzip", "x-gzip" -> runCatching {
            GzipSource(Buffer().write(bytes)).buffer().use { it.readByteArray() }
        }.getOrDefault(bytes)

        "deflate" -> runCatching {
            InflaterSource(Buffer().write(bytes), Inflater(true)).buffer()
                .use { it.readByteArray() }
        }.getOrDefault(bytes)

        else -> bytes
    }
}

private fun urlEncodeForm(s: String): String {
    val sb = StringBuilder(s.length)
    for (b in s.encodeToByteArray()) {
        val u = b.toInt() and 0xFF
        when {
            u in 'a'.code..'z'.code || u in 'A'.code..'Z'.code || u in '0'.code..'9'.code -> sb.append(u.toChar())
            u == ' '.code -> sb.append('+')
            u == '-'.code || u == '_'.code || u == '.'.code || u == '*'.code -> sb.append(u.toChar())
            else -> sb.append('%').append(u.toString(16).uppercase().padStart(2, '0'))
        }
    }
    return sb.toString()
}

private fun urlEncodeQuery(s: String): String {
    val sb = StringBuilder(s.length)
    for (b in s.encodeToByteArray()) {
        val u = b.toInt() and 0xFF
        when {
            u in 'a'.code..'z'.code || u in 'A'.code..'Z'.code || u in '0'.code..'9'.code -> sb.append(u.toChar())
            u == '-'.code || u == '_'.code || u == '.'.code || u == '~'.code -> sb.append(u.toChar())
            else -> sb.append('%').append(u.toString(16).uppercase().padStart(2, '0'))
        }
    }
    return sb.toString()
}

private inline fun String.ifNotEmpty(block: (String) -> Unit) {
    if (isNotEmpty()) block(this)
}

private const val DEFAULT_TIMEOUT_MS = 15_000L

private val RUST_OWNED_HEADERS = setOf(
    "connection", "content-length", "host", "keep-alive", "proxy-authorization",
    "proxy-connection", "te", "transfer-encoding", "upgrade",
)
