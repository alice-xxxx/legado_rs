package io.legado.sourceengine

import io.legado.app.model.analyzeRule.AnalyzeRuleCore
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.app.utils.KS_JSON
import io.legado.app.help.source.SourceNetworkProviders
import io.legado.sourceengine.host.*
import kotlinx.serialization.json.*
import java.io.IOException
import java.util.Base64
import kotlin.test.*

class HostCompatibilityTest {
    private class Host(val content:String="original",val header:String="inline; filename=\"流媒体.json\""):NativeHost("fixture",".") {
        var browserRequest:HostBrowserRequest?=null
        var browserBody="\"audio-url\""
        override fun http(request:String):String {
            val url=KS_JSON.parseToJsonElement(request).jsonObject.getValue("url")
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
        assertEquals("inline; filename=\"流媒体.json\"",response.headers()["Content-Disposition"])
        assertEquals(200,response.code())
    }

    @Test fun malformedHeadersRemainErrors() {
        val host=Host(header="bad\nheader")
        val rule=AnalyzeUrlCore("https://example.org/",coroutineContext=SourceEngineHostContext(host))
        assertFailsWith<IOException> { withoutStoredCookies { rule.getStrResponse() } }
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
}
