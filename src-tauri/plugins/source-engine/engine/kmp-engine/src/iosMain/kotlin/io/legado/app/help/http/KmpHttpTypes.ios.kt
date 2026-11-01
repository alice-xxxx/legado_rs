package io.legado.app.help.http

import okio.BufferedSource
import kotlin.reflect.KClass

actual typealias KmpInterceptor = HostInterceptor
actual typealias KmpInterceptorChain = HostInterceptorChain
actual typealias KmpHttpClient = HostHttpClient
actual typealias KmpHttpClientBuilder = HostHttpClientBuilder
actual typealias KmpRequest = HostRequest
actual typealias KmpRequestBuilder = HostRequestBuilder
actual typealias KmpResponse = HostResponse
actual typealias KmpResponseBuilder = HostResponseBuilder
actual typealias KmpResponseBody = HostResponseBody
actual typealias KmpCall = HostCall
actual typealias KmpCallback = HostCallback
actual typealias KmpFormBodyBuilder = HostFormBodyBuilder
actual typealias KmpHttpUrl = HostHttpUrl
actual typealias KmpHttpUrlBuilder = HostHttpUrlBuilder
actual typealias KmpMediaType = HostMediaType
actual typealias KmpRequestBody = HostRequestBody
actual typealias KmpHeaders = HostHeaders
actual typealias KmpProtocol = HostProtocol

actual fun KmpResponse.header(name: String, defaultValue: String?): String? = hostHeader(name, defaultValue)
internal actual fun KmpRequestBuilder.addNonAsciiRequestHeader(name: String, value: String): KmpRequestBuilder =
    addHostNonAsciiRequestHeader(name, value)
actual fun KmpFormBodyBuilder.buildKmpRequestBody(): KmpRequestBody = buildHostRequestBody()
actual fun String.toKmpHttpUrl(): KmpHttpUrl = toHostHttpUrl()
actual fun String.toKmpMediaType(): KmpMediaType = toHostMediaType()
actual fun String.toKmpRequestBody(contentType: KmpMediaType?): KmpRequestBody = toHostRequestBody(contentType)
actual fun ByteArray.toKmpRequestBody(contentType: KmpMediaType?): KmpRequestBody = toHostRequestBody(contentType)
actual fun <T : Any> KmpRequestBuilder.tagKmp(tagClass: KClass<T>, tag: T?): KmpRequestBuilder = tagHost(tagClass, tag)
actual fun <T : Any> KmpRequest.tagKmp(tagClass: KClass<T>): T? = tagHost(tagClass)
internal actual fun BufferedSource.asKmpResponseBody(contentType: KmpMediaType?, length: Long): KmpResponseBody =
    asHostResponseBody(contentType, length)
