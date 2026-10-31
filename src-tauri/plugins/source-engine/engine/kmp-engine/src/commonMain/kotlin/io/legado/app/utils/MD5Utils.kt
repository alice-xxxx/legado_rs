package io.legado.app.utils

import okio.Buffer
import okio.HashingSink
import okio.blackholeSink
import okio.ByteString.Companion.toByteString

object MD5Utils {
    fun md5Encode(str: String?): String = str?.let { md5Encode(it.encodeToByteArray()) }.orEmpty()

    fun md5Encode(bytes: ByteArray): String = bytes.toByteString().md5().hex()

    fun md5Encode(inputStream: InputStream): String {
        val digest = HashingSink.md5(blackholeSink())
        val buffer = Buffer()
        val bytes = ByteArray(8192)
        while (true) {
            val count = inputStream.read(bytes)
            if (count == -1) break
            buffer.write(bytes, 0, count)
            digest.write(buffer, count.toLong())
        }
        return digest.hash.hex()
    }

    fun md5Encode16(str: String): String = md5Encode(str).substring(8, 24)
}

fun ByteArray.toHexLower(): String = toByteString().hex()
