package io.legado.app.model.analyzeRule

import com.fleeksoft.ksoup.Ksoup
import kotlin.test.Test
import kotlin.test.assertEquals

class SelectorCompatibilityTest {
    private val html = """<main><div class="book"><a href="/one">One</a><span>Author <b>A</b></span><section><a href="/nested">Nested</a></section></div><div class="book"><a href="/two">Two</a></div></main>"""

    @Test fun textNodesAndChildSteps() {
        val rule = AnalyzeByXPath(html)
        assertEquals(listOf("One", "Two"), rule.getStringList("//div[@class='book']/a/text()"))
        assertEquals(listOf("Author ", "A"), rule.getStringList("//span//text()"))
        assertEquals(listOf("/one", "/two"), rule.getStringList("//div[@class='book']/a/@href"))
        assertEquals(listOf("A", "B", "C"), AnalyzeByXPath("<p>A<b>B</b>C</p>").getStringList("//p//text()"))
    }

    @Test fun chainedSelectorsDoNotDetachNodes() {
        val document=Ksoup.parse(html)
        val rule=AnalyzeByJSoup(document)
        repeat(3) { assertEquals(2,rule.getElements("class.book@tag.a.0").size) }
        assertEquals(2,document.select(".book").size)
        assertEquals(3,document.select("a").size)
    }

    @Test fun rowQueriesStayInsideTheSelectedBook() {
        val row = Ksoup.parse(html).select(".book")[1]
        assertEquals("Two", AnalyzeByXPath(row).getString("//a/text()"))
        assertEquals("/two", AnalyzeByXPath(row.selectFirst("a")!!).getString("//a/@href"))
    }

    @Test fun predicatesUseTheFilteredCandidatePositions() {
        val rule = AnalyzeByXPath("<ul><li>skip</li><li class='chapter'>One</li><li class='chapter'>Two</li></ul>")
        assertEquals("One", rule.getString("//li[@class='chapter'][1]/text()"))
        assertEquals("Two", rule.getString("//li[@class='chapter'][last()]/text()"))
        assertEquals("One", rule.getString("//li[@class='chapter'][last()-1]/text()"))
        assertEquals("Two", rule.getString("//li[contains(text(),'Two')]/text()"))
        assertEquals(listOf("One", "Two"), rule.getStringList("//li[@class='chapter']/text() | //li[@class='chapter']/text()"))
    }

    @Test fun elementResultsRetainImagesAndMarkup() {
        val rule = AnalyzeByXPath("<p>A<b>B</b></p><img src='/page.jpg'>")
        assertEquals("<p>A<b>B</b></p>", rule.getString("//p"))
        assertEquals(listOf("<img src=\"/page.jpg\">"), rule.getStringList("//img"))
    }

    @Test fun axesAndNodePredicatesUsedBySourceDirectories() {
        val rule = AnalyzeByXPath("<h1>Title</h1><ul><li><a>下一页</a></li><li><a href='/1'>One</a></li><li>Skip</li></ul><p>正文</p><footer><p>Footer</p></footer>")
        assertEquals("/1", rule.getString("//a[text()='下一页']/../following-sibling::li/a/@href"))
        assertEquals(listOf("下一页", "One"), rule.getStringList("//li[.//a]/a/text()"))
        assertEquals(listOf("正文"), rule.getStringList("//h1/following::p[not(ancestor::footer)]/text()"))
        assertEquals("下一页", rule.getString("//a[@href='/1']/../preceding-sibling::li[1]/a/text()"))
    }
}
