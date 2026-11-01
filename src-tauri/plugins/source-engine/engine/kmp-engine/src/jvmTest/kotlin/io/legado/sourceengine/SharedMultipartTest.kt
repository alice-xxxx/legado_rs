package io.legado.sourceengine

import io.legado.app.help.http.*
import io.legado.app.utils.toInputStream
import okhttp3.MultipartBody
import okio.Buffer
import org.jsoup.internal.HttpKeyVal
import org.jsoup.internal.buildMultipartBody
import java.io.ByteArrayInputStream
import java.io.IOException
import kotlin.test.*

class SharedMultipartTest {
    @Test fun nativeFallbackMatchesOkHttpForBinaryDataAndQuotedUnicodeNames() {
        val bytes = byteArrayOf(0, 13, 10, -1, -128, 42)
        val parts = listOf(
            MultipartPart("正文\"\r\n", null, "value😀\r\n".toKmpRequestBody(null)),
            MultipartPart("file", "文件\"\r\n.bin", bytes.toKmpRequestBody("application/octet-stream".toKmpMediaType())),
            MultipartPart("no-type", "none", byteArrayOf().toKmpRequestBody(null)),
        )
        val reference = MultipartBody.Builder("boundary").setType(MultipartBody.FORM).apply {
            parts.forEach { addFormDataPart(it.name, it.filename, it.body) }
        }.build()
        val expected = Buffer().also(reference::writeTo).readByteArray()
        val actual = Buffer().apply { writeMultipart(parts, "boundary", this) }.readByteArray()
        assertContentEquals(expected, actual)
    }

    @Test fun bothScriptMultipartEntrypointsPreserveFileBodiesAndBoundaries() {
        val bytes = byteArrayOf(0, -1, 1)
        val request = KmpRequestBuilder().url("https://fixture.invalid/").apply {
            postMultipart("multipart/form-data", linkedMapOf("text" to "正文", "upload" to mapOf(
                "fileName" to "文件.bin", "file" to bytes, "contentType" to "application/octet-stream")))
        }.build()
        val multipart = request.body as MultipartBody
        assertEquals("multipart/form-data", multipart.type.toString())
        assertEquals("正文", Buffer().also(multipart.part(0).body::writeTo).readUtf8())
        assertContentEquals(bytes, Buffer().also(multipart.part(1).body::writeTo).readByteArray())
        val jsoup = buildMultipartBody(listOf(HttpKeyVal.create("upload", "文件.bin", bytes.toInputStream())), "custom-boundary")
        assertEquals("multipart/form-data; boundary=custom-boundary", jsoup.contentType().toString())
        assertTrue(jsoup.isOneShot())
        assertTrue(Buffer().also(jsoup::writeTo).readUtf8().endsWith("--custom-boundary--\r\n"))
    }

    @Test fun streamPartsCloseOnSuccessAndPreserveReadFailures() {
        var closed = false
        val input = object : ByteArrayInputStream(ByteArray(20_000) { it.toByte() }) {
            override fun close() { closed = true; super.close() }
        }
        val body = streamMultipartBody(input, "application/octet-stream")
        assertFalse(closed)
        assertTrue(body.isOneShot())
        assertEquals(20_000L, Buffer().also(body::writeTo).size)
        assertTrue(closed)
        val error = IOException("fixture")
        closed = false
        val broken = object : java.io.InputStream() {
            override fun read(): Int = throw error
            override fun close() { closed = true }
        }
        assertSame(error, assertFailsWith<IOException> { streamMultipartBody(broken, "application/octet-stream").writeTo(Buffer()) })
        assertTrue(closed)
        assertFails { writeMultipart(emptyList(), "boundary", Buffer()) }
        assertFails { KmpRequestBuilder().postMultipart("text/plain", mapOf("key" to "value")) }
    }
}
