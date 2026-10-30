package io.legado.app.model.analyzeRule

import com.fleeksoft.ksoup.nodes.Element
import com.fleeksoft.ksoup.select.Elements
import io.legado.app.model.script.JavaClassCompat
import kotlin.test.*

class JavaClassCompatTest {
    private fun construct(name: String, vararg args: Any?) = JavaClassCompat.construct(JavaClassCompat.loadClass(name), args.toList())
    private fun call(value: Any, method: String, vararg args: Any?) = JavaClassCompat.call(value, method, args.toList())
    private fun static(name: String, method: String, vararg args: Any?) = JavaClassCompat.callStatic(JavaClassCompat.loadClass(name), method, args.toList())

    @Test fun cardsCanBeBuiltSelectedAndReturnedAsHtml() {
        val cards = construct("org.jsoup.select.Elements") as Elements
        val card = construct("org.jsoup.nodes.Element", "div") as Element
        val link = construct("org.jsoup.nodes.Element", "a")
        assertSame(link, call(link, "attr", "href", "/book"))
        assertSame(link, call(link, "text", "我的小说"))
        call(card, "appendChild", link)
        call(cards, "add", card)
        assertEquals(1, call(cards, "size"))
        assertSame(card, call(cards, "get", 0))
        val selected = call(cards, "select", "a")!!
        assertEquals("我的小说", call(selected, "text"))
        assertEquals("/book", call(selected, "attr", "href"))
        assertEquals(listOf("<div>\n <a href=\"/book\">我的小说</a>\n</div>"), call(cards, "toJSON"))
    }

    @Test fun clonesAndListClearingPreserveTheOriginalDocument() {
        val doc = static("org.jsoup.Jsoup", "parse", "<p>Intro<a href='/book'>Link</a></p>", "https://example.org/")!!
        val original = call(doc, "selectFirst", "p")!!
        val clone = call(original, "clone")!!
        call(call(clone, "select", "a")!!, "remove")
        assertEquals("Intro", call(clone, "text"))
        val links = call(original, "select", "a")!!
        assertEquals("https://example.org/book", call(call(links, "first")!!, "absUrl", "href"))
        call(links, "clear")
        assertEquals(0, call(links, "size"))
        assertEquals("IntroLink", call(original, "text"))
        assertNull(call(doc, "selectFirst", "missing"))
    }

    @Test fun bodyFragmentAndEntityParserUseTheSharedParser() {
        val doc = static("org.jsoup.Jsoup", "parseBodyFragment", "<div><img src='page.png'></div>")!!
        val box = call(call(doc, "body")!!, "child", 0)!!
        assertContains(call(box, "outerHtml").toString(), "<img src=\"page.png\">")
        assertEquals("A & B", static("org.jsoup.parser.Parser", "unescapeEntities", "A &amp; B", true))
    }

    @Test fun unsupportedCallsFailAtTheOriginalOperation() {
        assertEquals(0, JavaClassCompat.loadClass("java.lang.Runtime"))
        assertFailsWith<IllegalStateException> { JavaClassCompat.construct(0, emptyList()) }
        assertFailsWith<IllegalStateException> { call(construct("org.jsoup.nodes.Element", "div"), "missing") }
    }

    @Test fun javaStringConstructorsDecodeByteSlices() {
        assertEquals("Value", construct("java.lang.String", "Value".encodeToByteArray()))
        assertEquals("Value", construct("java.lang.String", "xValuey".encodeToByteArray(), 1, 5, "UTF-8"))
        assertEquals("", construct("java.lang.String", ByteArray(0), "UTF-8"))
    }
}
