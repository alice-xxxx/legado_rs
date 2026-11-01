package org.jsoup.internal

import io.legado.app.help.http.KmpHttpClient
import okhttp3.OkHttpClient
import okio.IOException
import java.io.UncheckedIOException
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.net.ssl.HostnameVerifier
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocketFactory
import javax.net.ssl.X509TrustManager

internal actual fun decompressBody(bytes: ByteArray, contentEncoding: String?): ByteArray =
    io.legado.app.help.http.decompressHttpBytes(bytes, contentEncoding)

internal actual fun buildPlatformClient(
    request: HttpConnectionRequest,
    shared: KmpHttpClient?,
): KmpHttpClient {
    // 宿主注入了共享 client 则 newBuilder 派生(继承拦截器/连接池),否则裸建
    val builder = (shared?.newBuilder() ?: OkHttpClient.Builder())
        .connectTimeout(request.timeout().toLong(), TimeUnit.MILLISECONDS)
        .readTimeout(request.timeout().toLong(), TimeUnit.MILLISECONDS)
        .followRedirects(request.followRedirects())
        .followSslRedirects(request.followRedirects())

    if (shared != null) {
        // 共享 client 带 callTimeout(15s),会盖过上面 per-request 的 connect/read 超时,清掉
        builder.callTimeout(0, TimeUnit.MILLISECONDS)
    } else {
        builder.hostnameVerifier(AllHostHostnameVerifier)
    }

    (request.sslSocketFactory() as? SSLSocketFactory)?.let { factory ->
        builder.sslSocketFactory(factory, UnsafeTrustManager)
    } ?: (request.sslContext() as? SSLContext)?.let { ctx ->
        builder.sslSocketFactory(ctx.socketFactory, UnsafeTrustManager)
    }

    return builder.build()
}

internal actual fun defaultSharedHttpClient(): KmpHttpClient? = null

internal actual fun uncheckedIoException(e: IOException): Throwable = UncheckedIOException(e)

private object AllHostHostnameVerifier : HostnameVerifier {
    override fun verify(hostname: String, session: javax.net.ssl.SSLSession): Boolean = true
}

private object UnsafeTrustManager : X509TrustManager {
    override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {}
    override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {}
    override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
}
