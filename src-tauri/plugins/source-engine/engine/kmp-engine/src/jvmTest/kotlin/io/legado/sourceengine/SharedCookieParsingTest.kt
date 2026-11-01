package io.legado.sourceengine

import io.legado.app.help.http.parseCompatCookies
import okhttp3.Cookie
import okhttp3.HttpUrl.Companion.toHttpUrl
import kotlin.test.*

class SharedCookieParsingTest {
    @Test fun cookieFallbackMatchesTheLibraryForExistingSupportedInputs() {
        val headers = listOf(
            "name=value", "empty=", "=empty", "missing", "a=b=c", "a=中文", "a=bad\u0001",
            " a = b ; Secure; HttpOnly", "a=b; Path=/", "a=b; Domain=example.org",
            "a=b; Domain=.example.org", "a=b; Domain=other.invalid", "a=b; Domain=example.org.",
            "a=b; Domain=127.0.0.1", "a=b; Max-Age=0",
            "a=b; Max-Age=-1", "a=b; Max-Age=30", "a=b; Max-Age=invalid",
            "a=b; Max-Age=9999999999999999999999999999999999",
            "a=b; Max-Age=-9999999999999999999999999999999999",
            "a=b; Expires=Wed, 09 Jun 2021 10:18:14 GMT",
            "a=b; Expires=Sunday, 06-Nov-94 08:49:37 GMT",
            "a=b; Expires=Sun Nov 6 08:49:37 1994", "a=b; Expires=bad",
            "a=b; Expires=Wed, 09 Jun 1599 10:18:14 GMT",
            "a=b; Expires=Wed, 09 Jun 2021 25:18:14 GMT",
            "a=b; Expires=bad; Max-Age=2", "a=b; Expires=Wed, 09 Jun 2021 10:18:14 GMT; Max-Age=bad"
        )
        for (url in listOf("https://example.org/", "https://sub.example.org/path", "http://127.0.0.1/", "http://[::1]/")) {
            for (header in headers) {
                val expected = listOfNotNull(Cookie.parse(url.toHttpUrl(), header)).map { Triple(it.name, it.value, it.persistent) }
                val actual = parseCompatCookies(url, mapOf("sEt-CoOkIe" to listOf(header)), ::Triple)
                assertEquals(expected, actual, url + " " + header)
            }
        }
    }

    @Test fun nativeFallbackKeepsItsExistingDomainAndDateAcceptance() {
        val url = "https://example.org/"
        val headers = mapOf("Set-Cookie" to listOf(
            "psl=1; Domain=org", "idn=1; Domain=例子.org", "invalid=1; Domain=bad_domain",
            "date=1; Expires=Wed, 31 Feb 2021 10:18:14 GMT"
        ))
        assertEquals(listOf(Triple("psl", "1", false), Triple("idn", "1", false), Triple("invalid", "1", false), Triple("date", "1", true)),
            parseCompatCookies(url, headers, ::Triple))
    }

    @Test fun responseHeadersKeepCookieOrderAndIgnoreOtherFields() {
        assertEquals(listOf(Triple("a", "1", false), Triple("a", "2", true), Triple("b", "3", false)),
            parseCompatCookies("https://user:pass@example.org:443/", linkedMapOf(
                "Content-Type" to listOf("text/plain"), "Set-Cookie" to listOf("a=1", "a=2; Max-Age=1"),
                "set-cookie" to listOf("b=3")), ::Triple))
        assertTrue(parseCompatCookies("invalid", mapOf("Set-Cookie" to listOf("a=1")), ::Triple).isEmpty())
    }
}
