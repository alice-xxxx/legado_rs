package io.legado.sourceengine

import io.legado.app.model.analyzeRule.AnalyzeRuleCore
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.app.utils.KS_JSON
import io.legado.app.utils.readAllAndClose
import io.legado.app.help.source.SourceNetworkProviders
import io.legado.sourceengine.host.*
import io.legado.sourceengine.api.NativeCodec
import io.legado.sourceengine.api.ReaderSource
import kotlinx.serialization.json.*
import java.io.IOException
import java.util.Base64
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlin.test.*

class HostCompatibilityTest {
    private class Host(val content:String="original",val header:String="inline; filename=\"流媒体.json\""):NativeHost("fixture",".") {
        override val isCancelled=false
        var browserRequest:HostBrowserRequest?=null
        var httpRequest:JsonObject?=null
        var browserBody="\"audio-url\""
        var beforeHttp:(()->Unit)?=null
        override fun http(request:String):String {
            beforeHttp?.invoke()
            httpRequest=KS_JSON.parseToJsonElement(request).jsonObject
            val url=httpRequest!!.getValue("url")
            return buildJsonObject {
                put("ok",true)
                putJsonObject("value") {
                    put("requestedUrl",url);put("finalUrl",url);put("status",200);put("reason","OK")
                    put("bodyBase64",Base64.getEncoder().encodeToString(content.toByteArray()))
                    putJsonArray("headers") { addJsonObject { put("name","Content-Disposition");put("value",header) } }
                }
            }.toString()
        }
        override fun store(request:String)= """{"ok":true,"value":{"value":null}}"""
        override suspend fun browser(request:String):String {
            val value=wire.decodeFromString<HostBrowserRequest>(request);browserRequest=value
            return buildJsonObject {
                put("ok",true)
                putJsonObject("value") { put("capabilityId",value.capabilityId);put("finalUrl",value.baseUrl);put("body",browserBody) }
            }.toString()
        }
    }
    init { HostRuntime.install { error("Image operation outside fixture") } }

    private fun <T> withoutStoredCookies(block:()->T):T {
        val previous=SourceNetworkProviders.impl
        SourceNetworkProviders.impl=null
        return try { block() } finally { SourceNetworkProviders.impl=previous }
    }

    @Test fun unicodeResponseHeadersAndBodyScripts() {
        val host=Host()
        val rule=AnalyzeUrlCore("""https://example.org/,{"bodyJs":"result.toUpperCase()"}""",coroutineContext=SourceEngineHostContext(host))
        val response=withoutStoredCookies { rule.getStrResponse() }
        assertEquals("ORIGINAL",response.body)
        val stream = withoutStoredCookies { rule.getInputStream() }
        assertEquals("original", stream.readAllAndClose().decodeToString())
        assertEquals("inline; filename=\"流媒体.json\"",response.headers()["Content-Disposition"])
        assertEquals(200,response.code())
    }

    @Test fun malformedHeadersRemainErrors() {
        val host=Host(header="bad\nheader")
        val rule=AnalyzeUrlCore("https://example.org/",coroutineContext=SourceEngineHostContext(host))
        assertFailsWith<IOException> { withoutStoredCookies { rule.getStrResponse() } }
    }

    @Test fun hostErrorsRemainRequestFailures() {
        val host=Host().apply { beforeHttp={throw AssertionError("fixture")} }
        val rule=AnalyzeUrlCore("https://example.org/",coroutineContext=SourceEngineHostContext(host))
        val error=assertFailsWith<IOException> { withoutStoredCookies { rule.getStrResponse() } }
        assertTrue(generateSequence<Throwable>(error) { it.cause }.any { it is AssertionError })
        host.beforeHttp=null
        assertEquals("original",withoutStoredCookies { rule.getStrResponse() }.body)
    }

    @Test fun webRulesUseBrowserResultsAndPreserveJsonTypes() {
        val host=Host()
        val rule=AnalyzeRuleCore().setContent("<audio></audio>","https://example.org/chapter")
        SourceEngineHostRegistry.onThread(host) {
            assertEquals("audio-url",rule.getString("@webjs:document.querySelector('audio').src"))
            assertEquals(HostBrowserMode.DYNAMIC_PAGE,host.browserRequest?.mode)
            assertEquals("https://example.org/chapter",host.browserRequest?.baseUrl)
            assertEquals(1000L,host.browserRequest?.delayTimeMs)
            host.browserBody="[{\"name\":\"One\"},{\"name\":\"Two\"}]"
            assertEquals(2,rule.getElements("@webjs:[]").size)
        }
    }

    @Test fun legacyWebViewUsesTheRegisteredBrowserForUrlsAndHtml() {
        val host=Host().apply { browserBody="legacy" }
        kotlinx.coroutines.runBlocking(SourceEngineHostContext(host)) {
            val factory=io.legado.app.help.http.BackstageWebViewProviders.get()
            assertEquals("legacy",factory.create(url="https://example.org/page",javaScript="document.title").getStrResponse().body)
            assertEquals("",host.browserRequest?.html)
            assertEquals(HostBrowserMode.DYNAMIC_PAGE,host.browserRequest?.mode)
            assertEquals("legacy",factory.create(url="https://example.org/page",html="<p>Inline</p>",javaScript="document.title",delayTime=20).getStrResponse().body)
            assertEquals("<p>Inline</p>",host.browserRequest?.html)
            assertEquals(1020L,host.browserRequest?.delayTimeMs)
            factory.create(url="https://example.org/page",overrideUrlRegex="pixiv:.*").getStrResponse()
            assertEquals("pixiv:.*",host.browserRequest?.overrideUrlRegex)
        }
    }

    @Test fun jsoupUsesTheSameHostTransportAndHeadersAsOtherRules() {
        val host=Host(content="<p>Host transport</p>")
        SourceEngineHostRegistry.onThread(host) {
            val response=org.jsoup.Jsoup.connect("https://host.invalid/").header("X-Test","verified").execute()
            assertEquals("Host transport",response.parse().text())
        }
        assertEquals("https://host.invalid/",host.httpRequest?.get("url")?.jsonPrimitive?.content)
        assertTrue(host.httpRequest!!.getValue("headers").jsonArray.any { it.jsonObject["name"]?.jsonPrimitive?.content=="X-Test" && it.jsonObject["value"]?.jsonPrimitive?.content=="verified" })
        val rule=AnalyzeRuleCore().setContent("", "https://host.invalid/")
        SourceEngineHostRegistry.onThread(host) {
            assertEquals("Host transport",rule.getString("@js:org.jsoup.Jsoup.connect('https://host.invalid/').get().select('p').text()"))
            assertEquals("Host transport",rule.getString("@js:org.jsoup.Jsoup.connect('https://host.invalid/').data('key','value').post().select('p').text()"))
        }
        assertEquals("POST",host.httpRequest?.get("method")?.jsonPrimitive?.content)
    }

    @Test fun legacyWebViewCacheFlagDoesNotBecomeANumericDelay() {
        val host=Host().apply { browserBody="legacy" }
        val rule=AnalyzeRuleCore().setContent("<p>fixture</p>","https://example.org/")
        SourceEngineHostRegistry.onThread(host) {
            for (flag in listOf(false,true)) {
                assertEquals("legacy",rule.getString("@js:java.webView(null,'https://example.org/','document.title',$flag)"))
                assertEquals(1000L,host.browserRequest?.delayTimeMs)
                assertEquals("legacy",rule.getString("@js:java.webViewGetOverrideUrl(null,'https://example.org/','','pixiv:.*',$flag,17)"))
                assertEquals(1017L,host.browserRequest?.delayTimeMs)
                assertEquals("pixiv:.*",host.browserRequest?.overrideUrlRegex)
            }
        }
    }

    @Test fun resultRulesAndSharedFunctionsReceiveTheCurrentSearchVariables() {
        val host=Host()
        fun request(keyword:String,page:Int)=NativeCodec.decodeSearchRequest("""{"source":{"bookSourceUrl":"https://example.org/","searchUrl":"https://example.org/?q={{key}}&page={{page}}","jsLib":"function label(){return key+'-'+page;}","ruleSearch":{"bookList":"@js:[{name:label(),url:'https://example.org/book'}]","name":"name","bookUrl":"url"}},"keyword":"$keyword","page":$page}""")
        kotlinx.coroutines.runBlocking {
            SourceEngineHostRegistry.withHost(host) {
                assertEquals("我的-2",ReaderSource.search(request("我的",2)).books.single().name)
                assertEquals("系统-3",ReaderSource.search(request("系统",3)).books.single().name)
            }
        }
    }

    @Test fun aBlockedScriptRequestDoesNotBlockOtherSourceHosts() = runBlocking {
        val entered=CountDownLatch(1);val release=CountDownLatch(1)
        val firstHost=Host(content="first").apply { beforeHttp={entered.countDown();check(release.await(10,TimeUnit.SECONDS))} }
        val secondHost=Host(content="second")
        fun request(origin:String)=NativeCodec.decodeSourceRequest("""{"source":{"bookSourceUrl":"$origin","exploreUrl":"@js:JSON.stringify([{title:java.ajax('https://host.invalid/'),url:'https://host.invalid/'}]);"}}""")
        val first=async(Dispatchers.Default) { SourceEngineHostRegistry.withHost(firstHost) { ReaderSource.exploreKinds(request("https://first.host.invalid/")) } }
        try {
            assertTrue(entered.await(5,TimeUnit.SECONDS))
            val second=withTimeout(5000) { SourceEngineHostRegistry.withHost(secondHost) { ReaderSource.exploreKinds(request("https://second.host.invalid/")) } }
            assertEquals("second",second.categories.single().title)
        } finally { release.countDown() }
        assertEquals("first",first.await().categories.single().title)
    }

    @Test fun detailInitializationPreservesVariablesForTheCatalogUrl() = runBlocking {
        val host=Host(content="""{"data":[{"original_book_name":"fixture","book_id":"12345"}]}""")
        val request=NativeCodec.decodeBookRequest("""{"source":{"bookSourceUrl":"https://detail.invalid/","ruleBookInfo":{"init":"data[0]","name":"original_book_name@put:{book_id:book_id}","tocUrl":"https://detail.invalid/catalog?bookId=@get:{book_id}"}},"book":{"bookUrl":"https://detail.invalid/book","name":"search name"}}""")
        SourceEngineHostRegistry.withHost(host) {
            val detail=ReaderSource.detail(request)
            assertEquals("https://detail.invalid/catalog?bookId=12345",detail.tocUrl)
            val restored=NativeCodec.decodeBookRequest("""{"source":{"bookSourceUrl":"https://detail.invalid/"},"book":${NativeCodec.encodeBook(detail)}}""")
            assertEquals("12345",restored.book.getVariable("book_id"))
        }
    }

    @Test fun sourceQuotaDoesNotAllowAnExtraRequest() = runBlocking {
        for (allowed in 1..3) {
            val source=io.legado.app.data.entities.BookSource(bookSourceUrl="https://quota-fixture.invalid/$allowed",concurrentRate="$allowed/60000")
            val limiter=io.legado.app.help.coroutine.ConcurrentRateLimiter(source)
            repeat(allowed) { limiter.getConcurrentRecord() }
            val next=async(start=CoroutineStart.UNDISPATCHED) { limiter.getConcurrentRecord() }
            try { assertFalse(next.isCompleted,"Quota $allowed allowed an extra request") }
            finally { next.cancelAndJoin() }
        }
    }
}
