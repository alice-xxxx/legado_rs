package io.legado.app.help.http

import io.legado.app.utils.Closeable
import io.legado.app.utils.InputStream
import okio.BufferedSink
import okio.BufferedSource
import kotlin.time.Duration

expect interface KmpInterceptor {
    fun intercept(chain: KmpInterceptorChain): KmpResponse
}

expect interface KmpInterceptorChain {
    fun request(): KmpRequest
    fun proceed(request: KmpRequest): KmpResponse
}

expect class KmpHttpClient {
    fun newCall(request: KmpRequest): KmpCall
    fun newBuilder(): KmpHttpClientBuilder
}

expect class KmpHttpClientBuilder() {
    fun readTimeout(duration: Duration): KmpHttpClientBuilder
    fun callTimeout(duration: Duration): KmpHttpClientBuilder
    fun build(): KmpHttpClient
}

expect class KmpRequest {
    fun newBuilder(): KmpRequestBuilder
    fun header(name: String): String?
    val url: KmpHttpUrl
    val method: String
}

expect class KmpRequestBuilder() {
    fun url(url: String): KmpRequestBuilder
    fun url(url: KmpHttpUrl): KmpRequestBuilder
    fun addHeader(name: String, value: String): KmpRequestBuilder
    fun header(name: String, value: String): KmpRequestBuilder
    fun removeHeader(name: String): KmpRequestBuilder
    fun get(): KmpRequestBuilder
    fun post(body: KmpRequestBody): KmpRequestBuilder
    fun method(method: String, body: KmpRequestBody?): KmpRequestBuilder
    fun build(): KmpRequest
}

expect class KmpResponse : Closeable {
    val code: Int
    val message: String
    val body: KmpResponseBody
    val isSuccessful: Boolean
    val request: KmpRequest
    val networkResponse: KmpResponse?
    val priorResponse: KmpResponse?
    val isRedirect: Boolean
    fun headers(): KmpHeaders

    fun headers(name: String): List<String>
    fun newBuilder(): KmpResponseBuilder
    override fun close()
}

expect fun KmpResponse.header(name: String, defaultValue: String? = null): String?

expect class KmpResponseBuilder() {
    fun code(code: Int): KmpResponseBuilder
    fun message(message: String): KmpResponseBuilder
    fun protocol(protocol: KmpProtocol): KmpResponseBuilder
    fun request(request: KmpRequest): KmpResponseBuilder
    fun removeHeader(name: String): KmpResponseBuilder
    fun body(body: KmpResponseBody): KmpResponseBuilder
    fun build(): KmpResponse
}

expect abstract class KmpResponseBody : Closeable {
    fun bytes(): ByteArray
    fun byteStream(): InputStream
    abstract fun contentType(): KmpMediaType?
    abstract fun source(): BufferedSource
    fun string(): String
    override fun close()

    companion object {
        val EMPTY: KmpResponseBody
    }
}

expect interface KmpCall {
    fun enqueue(responseCallback: KmpCallback)
    fun cancel()
    fun execute(): KmpResponse
}

expect interface KmpCallback {
    fun onFailure(call: KmpCall, e: okio.IOException)
    fun onResponse(call: KmpCall, response: KmpResponse)
}

expect class KmpFormBodyBuilder() {
    fun add(name: String, value: String): KmpFormBodyBuilder
    fun addEncoded(name: String, value: String): KmpFormBodyBuilder
}

expect fun KmpFormBodyBuilder.buildKmpRequestBody(): KmpRequestBody

expect class KmpHttpUrl {
    fun newBuilder(): KmpHttpUrlBuilder
}

expect class KmpHttpUrlBuilder {
    fun encodedQuery(encodedQuery: String?): KmpHttpUrlBuilder
    fun addQueryParameter(name: String, value: String?): KmpHttpUrlBuilder
    fun addEncodedQueryParameter(encodedName: String, encodedValue: String?): KmpHttpUrlBuilder
    fun build(): KmpHttpUrl
}

expect class KmpMediaType

expect fun KmpMediaType.charsetName(): String?

expect abstract class KmpRequestBody() {
    abstract fun contentType(): KmpMediaType?
    abstract fun writeTo(sink: BufferedSink)
    open fun isOneShot(): Boolean
}

expect class KmpHeaders {
    fun toMultimap(): Map<String, List<String>>
}

expect enum class KmpProtocol {
    HTTP_1_0, HTTP_1_1, SPDY_3, HTTP_2, H2_PRIOR_KNOWLEDGE, QUIC
}

expect fun String.toKmpHttpUrl(): KmpHttpUrl

expect fun String.toKmpMediaType(): KmpMediaType

expect fun String.toKmpRequestBody(contentType: KmpMediaType?): KmpRequestBody

expect fun ByteArray.toKmpRequestBody(contentType: KmpMediaType?): KmpRequestBody

expect fun <T : Any> KmpRequestBuilder.tagKmp(tagClass: kotlin.reflect.KClass<T>, tag: T?): KmpRequestBuilder

expect fun <T : Any> KmpRequest.tagKmp(tagClass: kotlin.reflect.KClass<T>): T?

internal expect fun BufferedSource.asKmpResponseBody(contentType: KmpMediaType?, length: Long): KmpResponseBody
