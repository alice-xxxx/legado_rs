// Host-backed HTTP compatibility types; platform adapters select these or OkHttp.
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
import io.legado.app.utils.formUrlEncode
import io.legado.app.utils.PercentCodec
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import io.legado.app.help.coroutine.runBlockingInScope
import kotlin.coroutines.EmptyCoroutineContext
import okio.IOException
import kotlin.reflect.KClass
import kotlin.time.Duration
import okio.Buffer
import okio.BufferedSink
import okio.BufferedSource
interface HostInterceptor {
    fun intercept(chain: HostInterceptorChain): HostResponse
}

interface HostInterceptorChain {
    fun request(): HostRequest
    fun proceed(request: HostRequest): HostResponse
}

class HostHttpClient internal constructor(builder: HostHttpClientBuilder?) {
    constructor() : this(null)
    internal val readTimeoutMillis = builder?.readTimeoutMillis?.takeIf { it > 0 } ?: 15_000L
    internal val callTimeoutMillis = builder?.callTimeoutMillis?.takeIf { it > 0 } ?: 15_000L
    internal val proxyUrl = builder?.proxyUrl
    fun newCall(request:HostRequest):HostCall=HostCallImpl(this,request)
    fun newBuilder()=HostHttpClientBuilder().also{
        it.readTimeoutMillis=readTimeoutMillis;it.callTimeoutMillis=callTimeoutMillis
        it.proxyUrl=proxyUrl
    }
}
class HostHttpClientBuilder constructor(){
    internal var readTimeoutMillis=0L
    internal var callTimeoutMillis=0L
    internal var proxyUrl:String?=null
    fun readTimeout(duration:Duration)=apply{readTimeoutMillis=duration.inWholeMilliseconds}
    fun callTimeout(duration:Duration)=apply{callTimeoutMillis=duration.inWholeMilliseconds}
    internal fun proxy(url:String)=apply{proxyUrl=url}
    fun build()=HostHttpClient(this)
}

class HostRequest internal constructor(
    internal val urlStr: String,
    val method: String = "GET",
    internal val headers: List<Pair<String, String>> = emptyList(),
    internal val body: HostRequestBody? = null,
    internal val tags: Map<KClass<*>, Any> = emptyMap()
) {
    constructor() : this("http://localhost/")
    fun newBuilder(): HostRequestBuilder {
        return HostRequestBuilder().also { b ->
            b.urlStr = urlStr
            b.method = method
            b.headers.addAll(headers)
            b.body = body
            b.tags.putAll(tags)
        }
    }

    fun header(name: String): String? =
        headers.firstOrNull { it.first.equals(name, ignoreCase = true) }?.second

    val url: HostHttpUrl
        get() = HostHttpUrl(urlStr)
}

class HostRequestBuilder constructor() {
    internal var urlStr: String = "http://localhost/"
    internal var method: String = "GET"
    internal val headers: MutableList<Pair<String, String>> = mutableListOf()
    internal var body: HostRequestBody? = null
    internal val tags: MutableMap<KClass<*>, Any> = LinkedHashMap()

    fun url(url: String): HostRequestBuilder {
        urlStr = url
        return this
    }

    fun url(url: HostHttpUrl): HostRequestBuilder {
        urlStr = url.urlStr ?: "http://localhost/"
        return this
    }

    fun addHeader(name: String, value: String): HostRequestBuilder {
        headers.add(name to value)
        return this
    }

    fun header(name: String, value: String): HostRequestBuilder {
        headers.removeAll { it.first.equals(name, ignoreCase = true) }
        headers.add(name to value)
        return this
    }

    fun removeHeader(name: String): HostRequestBuilder {
        headers.removeAll { it.first.equals(name, ignoreCase = true) }
        return this
    }

    fun get(): HostRequestBuilder {
        method = "GET"
        body = null
        return this
    }

    fun post(body: HostRequestBody): HostRequestBuilder {
        method = "POST"
        this.body = body
        return this
    }

    fun method(method: String, body: HostRequestBody?): HostRequestBuilder {
        this.method = method.uppercase()
        this.body = body
        return this
    }

    fun build(): HostRequest {
        return HostRequest(urlStr, method, headers.toList(), body, tags.toMap())
    }
}

internal fun HostRequestBuilder.addHostNonAsciiRequestHeader(
    name: String,
    value: String,
): HostRequestBuilder = addHeader(name, value)
class HostResponse internal constructor(
    val code: Int,
    val message: String = "OK",
    headers: Map<String, List<String>> = emptyMap(),
    body: ByteArray? = null,
    contentType: String? = null,
    val request: HostRequest = HostRequest(),
    redirects: List<HostHttpRedirect> = emptyList()
) : Closeable {
    constructor() : this(200)
    internal val headersVal = headers
    internal val bodyBytes = body
    internal val contentTypeStr = contentType
    internal var redirectsVal = redirects.toList()
    internal var priorResponseVal = buildPriorResponseChain(request, redirectsVal)

    private fun buildPriorResponseChain(
        request: HostRequest,
        redirects: List<HostHttpRedirect>,
    ): HostResponse? {
        var previous: HostResponse? = null
        redirects.forEachIndexed { index, redirect ->
            val prior = HostResponse(
                code = redirect.status,
                message = "",
                headers = mapOf("Location" to listOf(redirect.toUrl)),
                body = null,
                contentType = null,
                request = HostRequest(
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

    val body: HostResponseBody
        get() = HostByteResponseBody(bodyBytes ?: ByteArray(0), contentTypeStr)
    val isSuccessful: Boolean get() = code in 200..299
    val networkResponse: HostResponse? get() = null
    val priorResponse: HostResponse? get() = priorResponseVal
    val isRedirect: Boolean get() = code in 300..399

    fun headers(): HostHeaders = HostHeaders(headersVal)

    fun headers(name: String): List<String> =
        headersVal.entries.firstOrNull { it.key.equals(name, ignoreCase = true) }?.value
            ?: emptyList()

    fun newBuilder(): HostResponseBuilder {
        return HostResponseBuilder().also { b ->
            b.codeVal = code
            b.messageVal = message
            b.headersVal.putAll(headersVal)
            b.bodyBytes = bodyBytes
            b.contentTypeStr = contentTypeStr
            b.requestVal = request
            b.redirectsVal = redirectsVal.toList()
        }
    }

    override fun close() {
    }
}
fun HostResponse.hostHeader(name: String, defaultValue: String?): String? {
    return headersVal.entries.firstOrNull { it.key.equals(name, ignoreCase = true) }?.value?.firstOrNull()
        ?: defaultValue
}

class HostResponseBuilder constructor() {
    internal var codeVal: Int = 200
    internal var messageVal: String = "OK"
    internal val headersVal: MutableMap<String, List<String>> = LinkedHashMap()
    internal var bodyBytes: ByteArray? = null
    internal var contentTypeStr: String? = null
    internal var requestVal: HostRequest = HostRequest()
    internal var redirectsVal: List<HostHttpRedirect> = emptyList()

    fun code(code: Int): HostResponseBuilder {
        codeVal = code
        return this
    }

    fun message(message: String): HostResponseBuilder {
        messageVal = message
        return this
    }

    fun protocol(protocol: HostProtocol): HostResponseBuilder {
        return this
    }

    fun request(request: HostRequest): HostResponseBuilder {
        requestVal = request
        return this
    }

    fun removeHeader(name: String): HostResponseBuilder {
        headersVal.keys.removeAll { it.equals(name, ignoreCase = true) }
        return this
    }

    fun body(body: HostResponseBody): HostResponseBuilder {
        val nativeBody = body as? HostByteResponseBody
        bodyBytes = nativeBody?.bytesValue
        contentTypeStr = nativeBody?.contentTypeValue
        return this
    }

    fun build(): HostResponse {
        return HostResponse(
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

abstract class HostResponseBody : Closeable {
    fun bytes(): ByteArray = (this as HostByteResponseBody).bytesValue
    fun byteStream(): InputStream = (this as HostByteResponseBody).bytesValue.toInputStream()
    abstract fun contentType(): HostMediaType?
    abstract fun source(): BufferedSource
    fun string(): String = bytes().decodeToString()
    override fun close() {
    }

    companion object {
        val EMPTY: HostResponseBody
            get() = HostByteResponseBody(ByteArray(0), null)
    }
}

internal class HostByteResponseBody(
    internal val bytesValue: ByteArray,
    internal val contentTypeValue: String?
) : HostResponseBody() {
    override fun contentType(): HostMediaType? = contentTypeValue?.let { HostMediaTypeValue(it) }
    override fun source(): BufferedSource = Buffer().write(bytesValue)
}
internal class HostCallImpl(
    private val client: HostHttpClient,
    private val request: HostRequest
) : HostCall {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var job: Job? = null

    override fun enqueue(responseCallback: HostCallback) {
        job = scope.launch {
            try {
                val response = executeHost()
                responseCallback.onResponse(this@HostCallImpl, response)
            } catch (e: Throwable) {
                val ioException = if (e is okio.IOException) e else okio.IOException(e.message, e)
                runCatching { responseCallback.onFailure(this@HostCallImpl, ioException) }
            }
        }
    }

    override fun cancel() {
        job?.cancel()
        scope.cancel()
    }

    override fun execute(): HostResponse = runBlockingInScope(EmptyCoroutineContext) {
        executeHost()
    }

    private suspend fun executeHost(): HostResponse {
        val sourceHost = request.tagHost(SourceEngineHost::class) ?: SourceEngineHostRegistry.current()
        return executeRustHost(sourceHost ?: throw IOException("iOS source request has no active Rust host"))
    }

    private suspend fun executeRustHost(host: SourceEngineHost): HostResponse {
        val useCookieJar = request.header(cookieJarHeader) != null
        val prepared = request.prepareForSend()
        val body = (prepared.body as? HostByteRequestBody)?.bytes
        val hostHeaders = prepared.headers.mapNotNull { (name, value) ->
            if (name.lowercase() in RUST_OWNED_HEADERS || name.equals(cookieJarHeader, ignoreCase = true)) {
                null
            } else {
                HostHeader(name, value)
            }
        }.toMutableList()
        if (hostHeaders.none { it.name.equals("Content-Type", ignoreCase = true) }) {
            (prepared.body as? HostByteRequestBody)?.contentType?.let {
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
            HostRequest(response.finalUrl, prepared.method, prepared.headers, prepared.body, prepared.tags)
        } else {
            prepared
        }
        return HostResponse(
            code = response.status,
            message = response.reason,
            headers = groupedHeaders,
            body = runCatching { decompressHttpBytes(response.body, contentEncoding) }.getOrDefault(response.body),
            contentType = groupedHeaders.entries
                .firstOrNull { it.key.equals("Content-Type", ignoreCase = true) }
                ?.value?.firstOrNull(),
            request = finalRequest,
            redirects = response.redirects,
        )
    }

}

interface HostCall {
    fun enqueue(responseCallback: HostCallback)
    fun cancel()
    fun execute(): HostResponse
}

interface HostCallback {
    fun onFailure(call: HostCall, e: okio.IOException)
    fun onResponse(call: HostCall, response: HostResponse)
}
class HostFormBodyBuilder constructor() {
    private val entries: MutableList<Pair<String, String>> = mutableListOf()

    fun add(name: String, value: String): HostFormBodyBuilder {
        entries.add(name to value)
        return this
    }

    fun addEncoded(name: String, value: String): HostFormBodyBuilder {
        entries.add(name to value)
        return this
    }

    internal fun buildFormBody(): HostRequestBody {
        val sb = StringBuilder()
        entries.forEachIndexed { idx, (k, v) ->
            if (idx > 0) sb.append('&')
            sb.append(formUrlEncode(k)).append('=').append(formUrlEncode(v))
        }
        return HostByteRequestBody(
            sb.toString().encodeToByteArray(),
            HostMediaTypeValue("application/x-www-form-urlencoded")
        )
    }
}

fun HostFormBodyBuilder.buildHostRequestBody(): HostRequestBody = this.buildFormBody()

class HostHttpUrl {
    internal var urlStr: String? = null
        private set

    constructor()

    internal constructor(urlStr: String) {
        this.urlStr = urlStr
    }

    fun newBuilder(): HostHttpUrlBuilder {
        return HostHttpUrlBuilder(urlStr ?: "http://localhost/")
    }

    override fun toString(): String = urlStr ?: ""
}

class HostHttpUrlBuilder {
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

    fun encodedQuery(encodedQuery: String?): HostHttpUrlBuilder {
        encodedQueryStr = encodedQuery
        return this
    }

    fun addQueryParameter(name: String, value: String?): HostHttpUrlBuilder {
        queryParams.add(name to (value ?: ""))
        return this
    }

    fun addEncodedQueryParameter(encodedName: String, encodedValue: String?): HostHttpUrlBuilder {
        queryParams.add(encodedName to (encodedValue ?: ""))
        return this
    }

    fun build(): HostHttpUrl {
        val sb = StringBuilder(baseUrl)
        if (encodedQueryStr != null) {
            sb.append('?').append(encodedQueryStr)
        } else if (queryParams.isNotEmpty()) {
            sb.append('?')
            queryParams.forEachIndexed { idx, (k, v) ->
                if (idx > 0) sb.append('&')
                sb.append(PercentCodec.UNRESERVED.encode(k) { it.encodeToByteArray() }).append('=').append(PercentCodec.UNRESERVED.encode(v) { it.encodeToByteArray() })
            }
        }
        return HostHttpUrl(sb.toString())
    }
}

open class HostMediaType {
    internal var value: String = ""
        private set

    constructor()

    internal constructor(value: String) {
        this.value = value
    }

    override fun toString(): String = value
}

abstract class HostRequestBody constructor() {
    abstract fun contentType(): HostMediaType?
    abstract fun writeTo(sink: BufferedSink)
    open fun isOneShot(): Boolean = false
}

internal class HostByteRequestBody(
    internal val bytes: ByteArray,
    internal val contentType: HostMediaType?
) : HostRequestBody() {
    override fun contentType(): HostMediaType? = contentType
    override fun writeTo(sink: BufferedSink) { sink.write(bytes) }
}

class HostHeaders(private val map: Map<String, List<String>> = emptyMap()) {
    fun toMultimap(): Map<String, List<String>> = map
}

enum class HostProtocol {
    HTTP_1_0, HTTP_1_1, SPDY_3, HTTP_2, H2_PRIOR_KNOWLEDGE, QUIC
}
fun String.toHostHttpUrl(): HostHttpUrl = HostHttpUrl(this)

fun String.toHostMediaType(): HostMediaType = HostMediaTypeValue(this)

fun String.toHostRequestBody(contentType: HostMediaType?): HostRequestBody =
    HostByteRequestBody(this.encodeToByteArray(), contentType)

fun ByteArray.toHostRequestBody(contentType: HostMediaType?): HostRequestBody =
    HostByteRequestBody(this, contentType)

fun <T : Any> HostRequestBuilder.tagHost(tagClass: KClass<T>, tag: T?): HostRequestBuilder {
    if (tag == null) tags.remove(tagClass) else tags[tagClass] = tag
    return this
}

@Suppress("UNCHECKED_CAST")
fun <T : Any> HostRequest.tagHost(tagClass: KClass<T>): T? = tags[tagClass] as? T

internal fun BufferedSource.asHostResponseBody(contentType: HostMediaType?, length: Long): HostResponseBody =
    HostByteResponseBody(readByteArray(), contentType?.toString())
internal class HostMediaTypeValue(value: String) : HostMediaType(value)

private fun HostRequest.prepareForSend(): HostRequest {
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
        return builder.build()
    }
    return builder.build()
}

private inline fun String.ifNotEmpty(block: (String) -> Unit) {
    if (isNotEmpty()) block(this)
}

private const val DEFAULT_TIMEOUT_MS = 15_000L

private val RUST_OWNED_HEADERS = setOf(
    "connection", "content-length", "host", "keep-alive", "proxy-authorization",
    "proxy-connection", "te", "transfer-encoding", "upgrade",
)
