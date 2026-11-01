package io.legado.app.help.http

import okio.Buffer
import okio.BufferedSource
import okio.Inflater
import okio.InflaterSource
import okio.Source
import okio.buffer
import okio.use

internal fun KmpResponse.promisesBody(): Boolean {
    if (request.method == "HEAD") return false
    if (code !in 100..199 && code != 204 && code != 304) return true
    return (header("Content-Length")?.toLongOrNull() ?: -1L) != -1L ||
        header("Transfer-Encoding").equals("chunked", true)
}

internal expect fun gzipSource(source: BufferedSource): Source

internal fun contentDecodingSource(source: BufferedSource, encoding: String?): BufferedSource? =
    when (encoding?.lowercase()) {
        "gzip", "x-gzip" -> gzipSource(source).buffer()
        "deflate" -> InflaterSource(source, Inflater(true)).buffer()
        else -> null
    }

internal fun decompressHttpBytes(bytes: ByteArray, encoding: String?): ByteArray =
    contentDecodingSource(Buffer().write(bytes), encoding)?.use { it.readByteArray() } ?: bytes
