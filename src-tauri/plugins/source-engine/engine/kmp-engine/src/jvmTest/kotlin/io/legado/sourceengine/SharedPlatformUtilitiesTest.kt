package io.legado.sourceengine

import cn.hutool.core.codec.Base64
import io.legado.app.utils.*
import io.legado.app.help.http.text
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.ResponseBody.Companion.toResponseBody
import java.net.URLDecoder
import java.net.URLEncoder
import java.security.MessageDigest
import java.util.zip.GZIPInputStream
import kotlin.random.Random
import kotlin.test.*

class SharedPlatformUtilitiesTest {
    @Test fun gzipProducesACompleteStream() {
        for (text in listOf("", "\u6211\u7684\u7cfb\u7edf\uD83D\uDE00", "chapter".repeat(10000))) {
            val zipped = Base64Lenient.decode(gzipAndBase64Encode(text))
            assertEquals(text, GZIPInputStream(zipped.inputStream()).reader().readText())
        }
    }

    @Test fun formEncodingMatchesJdkAcrossCharsets() {
        for (charset in listOf("UTF-8", "UTF-16", "UTF-16LE", "UTF-32", "GBK", "GB18030", "Big5")) {
            for (text in listOf("", "a+b %/=?&*~\u6211\u7684", "\uD83D\uDE00\u0000\uD800")) {
                val expected = URLEncoder.encode(text, charset)
                assertEquals(expected, formUrlEncode(text, charset), charset)
                assertEquals(URLDecoder.decode(expected, charset), formUrlDecode(expected, charset), charset)
            }
        }
        for (bad in listOf("%", "%A", "%GG")) assertFails { formUrlDecode(bad) }
    }

    @Test fun streamingAndByteArrayDigestsMatchJdk() {
        for (size in listOf(0, 1, 63, 64, 65, 8192, 20000)) {
            val bytes = Random(size).nextBytes(size)
            val expected = MessageDigest.getInstance("MD5").digest(bytes).toHexLower()
            assertEquals(expected, MD5Utils.md5Encode(bytes))
            assertEquals(expected, MD5Utils.md5Encode(bytes.toInputStream()))
        }
    }

    @Test fun lenientBase64MatchesHutoolAndIgnoresJunk() {
        val random = Random(31)
        val fixtures = listOf("A", "AA", "AAA", "AAAAA", "Y=Q==Yg==", "-_+/", "{YQ}\u007F==") +
            List(2000) { random.nextBytes(random.nextInt(0, 512)).decodeToString() }
        for (input in fixtures) {
            // Hutool's table ends at 'z'; our existing codec also ignores the remaining ASCII junk.
            assertContentEquals(Base64.decode(input.filter { it.code !in 123..127 }), Base64Lenient.decode(input))
        }
    }

    @Test fun httpDecodingPrioritizesExplicitCharsetThenHeaderThenContent() {
        val html = "<head><meta charset=GBK></head><body>\u6211\u7684\u7cfb\u7edf</body>"
        val bytes = textCharsetCodec("GBK").encode(html)
        fun body(type: String) = bytes.toResponseBody(type.toMediaType())
        assertEquals(html, body("text/html; charset=UTF-8").text("GBK"))
        assertEquals(html, body("text/html; charset=GBK").text())
        assertEquals(html, body("text/html").text())
        assertEquals(html, body("text/html; charset=unknown-fixture").text())
        val utf8 = "\uFEFF\u6211\u7684".encodeToByteArray()
        assertEquals("\u6211\u7684", utf8.toResponseBody("text/plain; charset=UTF-8".toMediaType()).text())
    }

    @Test fun httpDecodingRejectsInvalidExplicitCharsetsAndFallsBackFromInvalidMeta() {
        val html = "<head><meta charset=unknown-fixture></head><body>\u6211\u7684</body>"
        fun body() = html.toByteArray().toResponseBody("text/html".toMediaType())
        assertFails { body().text("unknown-fixture") }
        assertEquals(html, body().text())
    }
}
