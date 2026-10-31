package io.legado.sourceengine

import io.legado.app.utils.NetworkUtils
import kotlin.test.*

class SharedNetworkUtilitiesTest {
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
