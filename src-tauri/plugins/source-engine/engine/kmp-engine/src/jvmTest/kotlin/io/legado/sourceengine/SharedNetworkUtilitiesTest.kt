package io.legado.sourceengine

import io.legado.app.utils.NetworkUtils
import io.legado.app.utils.ScriptUrlCompat
import io.legado.app.utils.URL
import java.net.URL as JavaURL
import kotlin.test.*

class SharedNetworkUtilitiesTest {
    @Suppress("DEPRECATION")
    @Test fun urlResolutionKeepsCommaOverloadsAndErrorFallbacks() {
        val base = "https://fixture.invalid/a,b/chapter?q=1#old"
        assertEquals("https://fixture.invalid/a,b/next", NetworkUtils.getAbsoluteURL(URL(base), "next"))
        assertEquals("https://fixture.invalid/next", NetworkUtils.getAbsoluteURL(base, "next"))
        assertEquals("next", NetworkUtils.getAbsoluteURL(null as String?, " next "))
        assertEquals("next", NetworkUtils.getAbsoluteURL("data:text/plain,base", " next "))
        assertEquals("next", NetworkUtils.getAbsoluteURL("not a URL", " next "))
        assertEquals("", NetworkUtils.getAbsoluteURL(base, "javascript:void(0)"))
        for (relative in listOf("next", "../next", "?new=2", "#new", "", "/a/../next", "//other.invalid/next")) {
            assertEquals(JavaURL(JavaURL(base), relative).toString(), NetworkUtils.getAbsoluteURL(URL(base), relative), relative)
        }
    }

    @Test fun domainHelpersKeepSuffixAndInvalidUrlFallbacks() {
        assertEquals("example.co.uk", NetworkUtils.getSubDomain("https://a.example.co.uk/path"))
        assertEquals("127.0.0.1", NetworkUtils.getSubDomain("http://127.0.0.1/path"))
        assertEquals("a.example.co.uk", NetworkUtils.getDomain("https://a.example.co.uk/path"))
        assertEquals("invalid", NetworkUtils.getSubDomain("invalid"))
        assertNull(NetworkUtils.getSubDomainOrNull("invalid"))
        assertEquals("invalid", NetworkUtils.getDomain("invalid"))
    }

    @Suppress("DEPRECATION")
    @Test fun sharedNativeFallbackMatchesJdkForSourceUrlComponentsAndRelativePaths() {
        val bases = listOf("https://fixture.invalid/a/b?q=1#old", "https://fixture.invalid", "https://fixture.invalid/",
            "https://fixture.invalid/a,b/c", "https://user:pass@fixture.invalid:8443/a/b", "https://[2001:db8::1]:8443/a/b")
        val relatives = listOf("", "next", ".", "..", "../next", "../../next", "../../../next", "./next", "next/..", "next/.",
            "/next/../part", "//other.invalid/a", "?new=2", "#new", "next?q=1#new")
        for (base in bases) for (relative in relatives) {
            val expected = JavaURL(JavaURL(base), relative)
            assertEquals(expected.toString(), ScriptUrlCompat.resolveRelative(base, relative), "$base + $relative")
            val actual = ScriptUrlCompat.parse(relative, base)
            assertEquals(expected.host, actual.host)
            assertEquals(expected.path, actual.pathname)
            assertEquals(expected.query, actual.query)
        }
    }

    @Test fun encodingChecksPreserveLegacyEscapesAndReservedCharacters() {
        for (value in listOf("", "abcXYZ019", "%20%00%fF", "a%2Bb")) {
            assertTrue(NetworkUtils.encodedQuery(value), value)
            assertTrue(NetworkUtils.encodedForm(value), value)
        }
        for (value in listOf("a+b=c&x=%00", "!$&()*+,-./:;=?@[\\]^_`{|}~", "a/b?q=x", "[::1]:80")) {
            assertTrue(NetworkUtils.encodedQuery(value), value)
            assertFalse(NetworkUtils.encodedForm(value), value)
        }
        for (value in listOf(" ", "\"", "#", "'", "%", "%A", "%GG", "%\uFF21\uFF22", "\u6211\u7684", "\uD83D\uDE00", "\u0000")) {
            assertFalse(NetworkUtils.encodedQuery(value), value)
            assertFalse(NetworkUtils.encodedForm(value), value)
        }
    }

    @Test fun ipChecksPreserveLegacyAddressRules() {
        for (value in listOf("127.0.0.1", "255.255.255.255")) {
            assertTrue(NetworkUtils.isIPv4Address(value), value)
            assertTrue(NetworkUtils.isIPAddress(value), value)
        }
        for (value in listOf("::", "::1", "2001:db8::1", "::ffff:192.0.2.1", "fe80::1%en0")) {
            assertTrue(NetworkUtils.isIPv6Address(value), value)
            assertTrue(NetworkUtils.isIPAddress(value), value)
        }
        for (value in listOf(null, "", "0.0.0.0", "01.2.3.4", "256.0.0.1", "1.2.3", "[::1]", "1:2:3:4:5:6:7:8:9", "example.org")) {
            assertFalse(NetworkUtils.isIPAddress(value), value)
        }
    }
}
