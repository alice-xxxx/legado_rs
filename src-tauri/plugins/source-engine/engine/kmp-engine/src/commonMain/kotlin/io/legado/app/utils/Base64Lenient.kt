package io.legado.app.utils

import okio.ByteString.Companion.decodeBase64
import okio.ByteString.Companion.toByteString

/** Legacy decoding ignores junk and padding, and accepts both Base64 alphabets. */
object Base64Lenient {
    fun decode(source: String): ByteArray = decode(source.encodeToByteArray())
    fun decodeStr(source: String?): String? = source?.let { decode(it).decodeToString() }

    fun decode(input: ByteArray): ByteArray {
        val clean = buildString(input.size) {
            for (byte in input) {
                val char = byte.toInt().toChar()
                if (char in 'A'..'Z' || char in 'a'..'z' || char in '0'..'9' || char in "+/-_") append(char)
            }
        }
        // A trailing sextet cannot form a byte; Hutool silently discards it.
        val complete = if (clean.length % 4 == 1) clean.dropLast(1) else clean
        return complete.decodeBase64()?.toByteArray() ?: ByteArray(0)
    }

    fun encodeToString(input: ByteArray): String = input.toByteString().base64()
}
