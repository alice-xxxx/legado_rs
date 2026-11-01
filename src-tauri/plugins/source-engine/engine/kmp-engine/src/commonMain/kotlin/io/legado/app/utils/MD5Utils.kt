package io.legado.app.utils

import okio.buffer
import okio.use
import okio.HashingSink
import okio.blackholeSink
import okio.ByteString.Companion.toByteString

object MD5Utils {
    fun md5Encode(str: String?): String = str?.let { md5Encode(it.encodeToByteArray()) }.orEmpty()

    fun md5Encode(bytes: ByteArray): String = bytes.toByteString().md5().hex()

    fun md5Encode(inputStream: InputStream): String {
        val digest = HashingSink.md5(blackholeSink())
        digest.buffer().use { it.writeAll(inputStream.asSource()) }
        return digest.hash.hex()
    }

    fun md5Encode16(str: String): String = md5Encode(str).substring(8, 24)
}

fun ByteArray.toHexLower(): String = toByteString().hex()
