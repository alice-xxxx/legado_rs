package io.legado.app.model.analyzeRule

import com.fleeksoft.io.ByteArrayInputStream
import io.legado.app.utils.InputStream
import io.legado.app.utils.readAllAndClose
import io.legado.app.utils.textCharsetCodec
import kotlin.test.*

class SharedCodecLibraryTest {
    @Test fun charsetSlicesAndMalformedInput() {
        for (charset in listOf("UTF-8", "UTF-16", "UTF-16LE", "UTF-16BE", "UTF-32", "UTF-32LE", "UTF-32BE", "GBK", "GB2312", "Big5", "GB18030", " GB_18030 ", "gb-18030")) {
            val codec = textCharsetCodec(charset)
            val text = "\u4e2d\u6587"
            val bytes = codec.encode(text)
            assertEquals(text, codec.decode(byteArrayOf(0) + bytes + byteArrayOf(0), 1, bytes.size), charset)
        }
        assertEquals("\uFFFD", textCharsetCodec("UTF-16BE").decode(byteArrayOf(0xD8.toByte(), 0)))
        assertEquals("\uFFFD", textCharsetCodec("UTF-8").decode(byteArrayOf(0xFF.toByte())))
        assertFails { textCharsetCodec("not-a-charset") }
    }

    @Test fun byteStreamsPreserveJdkBoundarySemantics() {
        val stream = ByteArrayInputStream(byteArrayOf(1, 2, 3))
        assertEquals(0, stream.read(ByteArray(0)))
        assertEquals(1L, stream.skip(1))
        assertEquals(2, stream.available())
        stream.mark(2)
        assertEquals(2, stream.read())
        stream.reset()
        assertContentEquals(byteArrayOf(2, 3), stream.readAllAndClose())
        assertEquals(-1, stream.read())
        assertEquals(-1, stream.read(ByteArray(0)))
        stream.close()
    }

    @Test fun readingClosesStreamsOnSuccessAndFailure() {
        val expected = ByteArray(20000) { it.toByte() }
        var closed = false
        val stream = object : ByteArrayInputStream(expected) {
            override fun close() { closed = true }
        }
        assertContentEquals(expected, stream.readAllAndClose())
        assertTrue(closed)

        closed = false
        val failure = IllegalStateException("read fixture")
        val broken = object : InputStream() {
            override fun read(): Int = throw failure
            override fun close() { closed = true; error("close fixture") }
        }
        assertSame(failure, assertFails { broken.readAllAndClose() })
        assertTrue(closed)
    }
}
