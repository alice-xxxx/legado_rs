package io.legado.sourceengine

import io.legado.app.utils.JsURL
import io.legado.app.utils.scriptUrlQuery
import kotlin.test.*

class SharedUrlQueryTest {
    @Test fun queryValuesPreserveEmptyKeysUnicodeAndLastDuplicate() {
        assertNull(scriptUrlQuery(null))
        assertEquals(emptyMap(), scriptUrlQuery(""))
        assertEquals(mapOf("" to "empty", "a" to "last", "b" to "a + 😀", "%61" to "raw"),
            scriptUrlQuery("=empty&a=first&ignored&b=a+%2B+😀&a=last&%61=raw"))
        assertEquals(mapOf("key" to "a=b"), scriptUrlQuery("key=a%3Db"))
    }

    @Test fun invalidEscapesFailWithoutBreakingLaterUrls() {
        for (query in listOf("a=%", "a=%A", "a=%GG")) assertFails { scriptUrlQuery(query) }
        assertEquals(mapOf("a" to "正文😀"), JsURL("https://fixture.invalid/?a=%E6%AD%A3%E6%96%87😀").searchParams)
    }

    @Test fun relativeUrlsKeepTheExistingJdkResolutionRules() {
        val base = "https://fixture.invalid/a/chapter?old=1#old"
        assertEquals("/a/", JsURL("?new=2", base).pathname)
        assertEquals(mapOf("new" to "2"), JsURL("?new=2", base).searchParams)
        assertEquals("/a/chapter", JsURL("#new", base).pathname)
        assertEquals("/next", JsURL("../next", base).pathname)
        assertEquals("other.invalid", JsURL("//other.invalid/path", base).host)
        assertEquals("https://fixture.invalid:8443", JsURL("https://fixture.invalid:8443").origin)
    }
}
