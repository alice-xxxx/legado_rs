package io.legado.sourceengine

import cn.hutool.crypto.digest.DigestUtil
import cn.hutool.crypto.digest.HMac
import io.legado.app.help.JsCryptoProviderJvm
import io.legado.app.model.analyzeRule.AnalyzeRuleCore
import io.legado.app.utils.Base64Lenient
import io.legado.sourceengine.host.HostRuntime
import kotlin.test.*

class SharedScriptUtilitiesTest {
    init { HostRuntime.install { error("Unexpected image operation") } }

    @Test fun digestAndHmacTextMatchThePreviousHutoolImplementation() {
        val provider = JsCryptoProviderJvm
        for (text in listOf("", "中文\u0000😀", "\uD800tail", "chapter".repeat(2000))) {
            assertEquals(DigestUtil.md5Hex(text), provider.md5Encode(text))
            assertEquals(DigestUtil.md5Hex(text).substring(8, 24), provider.md5Encode16(text))
            for (algorithm in listOf("MD5", "SHA1", "sha-256", "SHA-384", "SHA-512")) {
                val digest = DigestUtil.digester(algorithm)
                assertEquals(digest.digestHex(text), provider.digestHex(text, algorithm))
                assertEquals(Base64Lenient.encodeToString(digest.digest(text)), provider.digestBase64Str(text, algorithm))
            }
            for (algorithm in listOf("HmacMD5", "HmacSHA1", "hmacsha256", "HmacSHA384", "HmacSHA512")) {
                val key = "密钥\u0000😀\uD800"
                val hmac = HMac(algorithm, key.toByteArray())
                assertEquals(hmac.digestHex(text), provider.HMacHex(text, algorithm, key))
                assertEquals(Base64Lenient.encodeToString(hmac.digest(text)), provider.HMacBase64(text, algorithm, key))
            }
        }
    }

    @Test fun scriptCryptoErrorsRemainIsolated() {
        val rule = AnalyzeRuleCore()
        for (script in listOf("java.digestHex('text','invalid')", "java.HMacHex('text','HmacSHA256','')")) {
            assertFails { rule.evalJS(script) }
            assertEquals("900150983cd24fb0d6963f7d28e17f72", rule.evalJS("java.digestHex('abc','MD5')"))
            assertEquals("kAFQmDzST7DWlj99KOF/cg==", rule.evalJS("java.digestBase64Str('abc','MD5')"))
        }
    }

    @Test fun jsoupStaticMethodsRemainCallableThroughTheScriptBridge() {
        val rule = AnalyzeRuleCore()
        assertEquals("https://fixture.invalid/chapter", rule.evalJS("org.jsoup.Jsoup.parse('<a href=chapter>正文</a>','https://fixture.invalid/').select('a').attr('abs:href')"))
        assertEquals("正文", rule.evalJS("org.jsoup.Jsoup.parseBodyFragment('<p>正文</p>').text()"))
        assertEquals("正文", rule.evalJS("org.jsoup.Jsoup.parse('<p>正文</p>').text()"))
        assertEquals("正文", rule.evalJS("org.jsoup.Jsoup.parseBodyFragment('<p>正文</p>','https://fixture.invalid/').text()"))
    }
}
