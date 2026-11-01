package io.legado.sourceengine

import io.legado.app.help.UserAgentProvider
import io.legado.app.help.UserAgentProviders
import io.legado.app.help.http.*
import io.legado.sourceengine.host.*
import okio.Buffer
import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.test.*
import kotlin.time.Duration.Companion.seconds

class SharedHostHttpTest {
    private class Host : SourceEngineHost {
        override val executionId = "fixture"
        override val dataDirectory = "."
        override val storage: HostStorage get() = error("Unexpected storage operation")
        var request: HostHttpRequest? = null
        var failure: Throwable? = null
        var response: (HostHttpRequest) -> HostHttpResponse = { HostHttpResponse(it.url, it.url, 200, "OK", body = "next".toByteArray()) }
        override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse {
            this.request = request
            failure?.let { throw it }
            return response(request)
        }
        override suspend fun executeBrowser(request: HostBrowserRequest): HostBrowserResponse = error("Unexpected browser operation")
    }

    @Test fun hostRequestConversionPreservesHeadersBodyTimeoutsAndProxy() {
        val host = Host()
        val bytes = byteArrayOf(0, -1, 13, 10)
        val request = HostRequestBuilder().url("https://example.org/start")
            .post(bytes.toHostRequestBody("application/octet-stream".toHostMediaType()))
            .addHeader("Connection", "close").addHeader("Host", "ignored")
            .addHeader("Content-Length", "999").addHeader("Proxy-Authorization", "ignored")
            .addHeader("X-Source", "正文").addHeader(cookieJarHeader, "true")
            .tagHost(SourceEngineHost::class, host).build()
        val previous = UserAgentProviders.impl
        UserAgentProviders.impl = UserAgentProvider { "fixture UA" }
        try {
            HostHttpClientBuilder().readTimeout(2.seconds).callTimeout(3.seconds)
                .proxy("http://proxy.invalid:8080").build().newCall(request).execute().close()
        } finally { UserAgentProviders.impl = previous }
        val sent = host.request!!
        assertEquals("POST", sent.method)
        assertContentEquals(bytes, sent.body)
        assertEquals(listOf(HostHeader("X-Source", "正文"), HostHeader("User-Agent", "fixture UA"),
            HostHeader("Cache-Control", "no-cache"), HostHeader("Accept-Encoding", "gzip, deflate"),
            HostHeader("Content-Type", "application/octet-stream")), sent.headers)
        assertEquals(15_000L, sent.connectTimeoutMs)
        assertEquals(2_000L, sent.readTimeoutMs)
        assertEquals(3_000L, sent.callTimeoutMs)
        assertEquals("http://proxy.invalid:8080", sent.proxy)
        assertEquals(20, sent.maxRedirects)
        assertTrue(sent.useCookieJar)
        assertEquals("example.org", sent.cookieScope)
        assertTrue(sent.acceptInvalidCerts)
        assertEquals("close", request.header("Connection"))
    }

    @Test fun responsesPreserveBinaryBodiesHeadersAndRedirectHistory() {
        val host = Host()
        val bytes = byteArrayOf(0, -1, 1)
        host.response = { HostHttpResponse(it.url, "https://example.org/end", 206, "Partial Content",
            redirects = listOf(HostHttpRedirect(it.url, "https://example.org/middle", 301),
                HostHttpRedirect("https://example.org/middle", "https://example.org/end", 302)),
            headers = listOf(HostHeader("X-Test", "a"), HostHeader("X-Test", "b")), body = bytes) }
        val request = HostRequestBuilder().url("https://example.org/start").tagHost(SourceEngineHost::class, host).build()
        val response = HostHttpClientBuilder().build().newCall(request).execute()
        assertEquals(206, response.code)
        assertEquals("Partial Content", response.message)
        assertEquals("https://example.org/end", response.request.url.toString())
        assertEquals(listOf("a", "b"), response.headers("x-test"))
        assertContentEquals(bytes, response.body.bytes())
        assertContentEquals(bytes, response.body.source().readByteArray())
        assertEquals(302, response.priorResponse!!.code)
        assertEquals("https://example.org/middle", response.priorResponse!!.request.url.toString())
        assertEquals(301, response.priorResponse!!.priorResponse!!.code)
        assertContentEquals(bytes, response.newBuilder().build().body.bytes())
    }

    @Test fun nativeBuilderAndHeaderPoliciesRemainUnchanged() {
        val host = Host()
        val request = HostRequestBuilder().url("https://example.org/")
            .addHeader("User-Agent", "null").addHeader("Range", "bytes=1-")
            .tagHost(SourceEngineHost::class, host).build()
        HostHttpClientBuilder().build().newCall(request).execute()
        assertFalse(host.request!!.headers.any { it.name == "User-Agent" || it.name == "Accept-Encoding" })
        assertEquals(15_000L, host.request!!.readTimeoutMs)
        assertFalse(host.request!!.useCookieJar)
        assertNull(host.request!!.cookieScope)
        assertNull(request.newBuilder().tagHost(SourceEngineHost::class, null).build().tagHost(SourceEngineHost::class))
        val form = HostFormBodyBuilder().add("a b", "+正文").buildHostRequestBody()
        assertEquals("a+b=%2B%E6%AD%A3%E6%96%87", Buffer().also(form::writeTo).readUtf8())
        assertEquals("https://example.org/?a=x%2520y&b=", "https://example.org/?a=x%20y".toHostHttpUrl()
            .newBuilder().addQueryParameter("b", null).build().toString())
        assertEquals("https://example.org/?raw=%20", "https://example.org/".toHostHttpUrl()
            .newBuilder().encodedQuery("raw=%20").build().toString())
    }

    @Test fun callbackFailuresStayIsolatedAndSubsequentRequestsWork() {
        val host = Host().apply { failure = AssertionError("fixture") }
        val request = HostRequestBuilder().url("https://example.org/").tagHost(SourceEngineHost::class, host).build()
        val client = HostHttpClientBuilder().build()
        assertSame(host.failure, assertFailsWith<AssertionError> { client.newCall(request).execute() })
        val completed = CountDownLatch(1)
        var failure: IOException? = null
        client.newCall(request).enqueue(object : HostCallback {
            override fun onResponse(call: HostCall, response: HostResponse) { completed.countDown() }
            override fun onFailure(call: HostCall, e: IOException) {
                failure = e
                completed.countDown()
                error("callback failure")
            }
        })
        assertTrue(completed.await(5, TimeUnit.SECONDS))
        assertSame(host.failure, failure?.cause)
        host.failure = null
        assertEquals("next", client.newCall(request).execute().body.string())
        assertFailsWith<IOException> { client.newCall(request.newBuilder().tagHost(SourceEngineHost::class, null).build()).execute() }
    }
}
