package io.legado.app.model.analyzeRule

import io.legado.app.lib.icu4j.CharsetDetector
import io.legado.app.utils.detectCharsetName
import io.legado.app.utils.textCharsetCodec
import kotlin.test.*

class SharedCharsetDetectorTest {
    @Test fun unicodeAndChineseTextUseTheSameDetector() {
        val simplified = "我的小说，系统重生。这个故事发生在中国，阅读文字和章节内容。\n".repeat(30)
        val traditional = "我的小說，系統重生。這個故事發生在臺灣，閱讀文字和章節內容。\n".repeat(30)
        for ((charset, text) in listOf("UTF-8" to simplified, "GB18030" to simplified, "Big5" to traditional)) {
            val bytes = textCharsetCodec(charset).encode(text)
            assertEquals(charset, detectCharsetName(bytes), charset)
            assertEquals(text, CharsetDetector().setText(bytes).detect()!!.string)
        }
    }

    @Test fun unicodeBomAndHtmlFilteringPreserveDetection() {
        val text = "我的系统章节内容".repeat(10)
        for (charset in listOf("UTF-16LE", "UTF-16BE", "UTF-32LE", "UTF-32BE")) {
            val bytes = textCharsetCodec(charset).encode("\uFEFF$text")
            assertEquals(charset, detectCharsetName(bytes), charset)
        }
        val detector = CharsetDetector().apply { enableInputFilter(true) }
        assertEquals("UTF-8", detector.setText("<html><body>$text</body></html>".encodeToByteArray()).detect()!!.name)
    }
}
