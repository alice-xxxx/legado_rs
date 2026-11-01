package io.legado.app.utils

import okio.Buffer
import okio.Source
import okio.Timeout

/** Fleeksoft 输入流到 Okio 的公共适配，调用方决定是否关闭输入。 */
internal fun InputStream.asSource(): Source = object : Source {
    private val bytes = ByteArray(8192)

    override fun read(sink: Buffer, byteCount: Long): Long {
        require(byteCount >= 0)
        if (byteCount == 0L) return 0
        val count = read(bytes, 0, minOf(byteCount, bytes.size.toLong()).toInt())
        if (count < 0) return -1
        if (count > 0) {
            sink.write(bytes, 0, count)
            return count.toLong()
        }
        val byte = read()
        if (byte < 0) return -1
        sink.writeByte(byte)
        return 1
    }

    override fun timeout(): Timeout = Timeout.NONE
    override fun close() = this@asSource.close()
}
