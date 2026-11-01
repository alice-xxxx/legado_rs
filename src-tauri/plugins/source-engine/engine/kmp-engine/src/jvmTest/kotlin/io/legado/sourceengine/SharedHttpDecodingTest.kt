package io.legado.sourceengine

import io.legado.app.help.http.*
import io.legado.sourceengine.host.*
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.internal.http.promisesBody as okhttpPromisesBody
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.URLEncoder
import java.nio.charset.Charset
import java.util.zip.Deflater
import java.util.zip.DeflaterOutputStream
import java.util.zip.GZIPOutputStream
import kotlin.test.*

class SharedHttpDecodingTest {
    init { HostRuntime.install { error("Unexpected image operation") } }

    private fun gzip(bytes: ByteArray): ByteArray = ByteArrayOutputStream().apply {
        GZIPOutputStream(this).use { it.write(bytes) }
    }.toByteArray()

    private class Host(var bytes: ByteArray, val encoding: String) : SourceEngineHost {
        override val executionId = "decoding"
        override val dataDirectory = "."
        override val storage: HostStorage get() = error("Unexpected storage")
        var request: HostHttpRequest? = null
        override suspend fun executeBrowser(request: HostBrowserRequest): HostBrowserResponse = error("Unexpected browser")
        override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse {
            this.request = request
            return HostHttpResponse(request.url, request.url, 200, "OK",
                headers = listOf(HostHeader("Content-Encoding", encoding)), body = bytes)
        }
    }

    @Test fun gzipAndRawDeflateUseSharedDecodingAndKeepConcatenatedGzip() {
        val bytes = "正文😀\u0000".repeat(10_000).toByteArray()
        for (encoding in listOf("gzip", "x-gzip", "GZIP")) {
            assertContentEquals(bytes, decompressHttpBytes(gzip(bytes), encoding))
        }
        assertContentEquals(bytes + bytes, decompressHttpBytes(gzip(bytes) + gzip(bytes), "gzip"))
        val deflater = Deflater(Deflater.DEFAULT_COMPRESSION, true)
        val deflated = try {
            ByteArrayOutputStream().apply { DeflaterOutputStream(this, deflater).use { it.write(bytes) } }.toByteArray()
        } finally { deflater.end() }
        assertContentEquals(bytes, decompressHttpBytes(deflated, "deflate"))
        assertSame(bytes, decompressHttpBytes(bytes, "identity"))
        assertFails { decompressHttpBytes(byteArrayOf(1, 2, 3), "gzip") }
        assertContentEquals(bytes, decompressHttpBytes(gzip(bytes), "gzip"))
    }

    @Test fun responseBodyRulesMatchOkHttpAcrossMethodsStatusesAndHeaders() {
        for (method in listOf("GET", "HEAD")) for (code in listOf(100, 199, 200, 204, 205, 304, 500)) {
            for (length in listOf(null, "0", "-1", "-2", "broken")) for (transfer in listOf(null, "chunked", "CHUNKED")) {
                val response = Response.Builder().request(Request.Builder().url("https://fixture.invalid/").method(method, null).build())
                    .protocol(Protocol.HTTP_1_1).code(code).message("fixture")
                    .apply { length?.let { header("Content-Length", it) }; transfer?.let { header("Transfer-Encoding", it) } }.build()
                assertEquals(response.okhttpPromisesBody(), response.promisesBody(), "$method $code $length $transfer")
            }
        }
    }

    @Test fun hostResponsesRespectExplicitEncodingsAndRangesAndRecoverFromCorruptBodies() {
        val bytes = "正文😀".toByteArray()
        val compressed = gzip(bytes)
        val host = Host(compressed, "gzip")
        SourceEngineHostRegistry.onThread(host) {
            val client = OkHttpClientProviders.get().okHttpClient
            fun request(configure: KmpRequestBuilder.() -> Unit = {}) = client.newCall(
                KmpRequestBuilder().url("https://fixture.invalid/").apply(configure).build()).execute()
            request().use { response ->
                assertContentEquals(bytes, response.body.bytes())
                assertNull(response.header("Content-Encoding"))
                assertNull(response.header("Content-Length"))
            }
            assertTrue(host.request!!.headers.any { it.name == "Accept-Encoding" && it.value == "gzip, deflate" })
            for (configure in listOf<KmpRequestBuilder.() -> Unit>(
                { header("Accept-Encoding", "gzip") }, { header("Range", "bytes=0-10") })) {
                request(configure).use { assertContentEquals(compressed, it.body.bytes()) }
            }
            host.bytes = byteArrayOf(1, 2, 3)
            assertFailsWith<IOException> { request().use { it.body.bytes() } }
            host.bytes = compressed
            request().use { assertContentEquals(bytes, it.body.bytes()) }
        }
    }

    @Test fun jsoupFormBodiesKeepTheDeclaredCharset() {
        val host = Host("<p>ok</p>".toByteArray(), "identity")
        SourceEngineHostRegistry.onThread(host) {
            for (charset in listOf("UTF-8", "UTF-16", "GBK")) {
                val name = "键 +*"
                val value = "正文😀 plus +"
                org.jsoup.Jsoup.connect("https://fixture.invalid/").postDataCharset(charset).data(name, value).post()
                val expected = URLEncoder.encode(name, charset) + "=" + URLEncoder.encode(value, charset)
                assertContentEquals(expected.toByteArray(Charset.forName(charset)), host.request!!.body, charset)
            }
        }
    }
}
