package android.util

import io.legado.app.utils.EncoderUtils

/** Android's source-script API on desktop; Android builds use the platform class. */
object Base64 {
    const val DEFAULT = 0
    const val NO_PADDING = 1
    const val NO_WRAP = 2
    const val CRLF = 4
    const val URL_SAFE = 8
    const val NO_CLOSE = 16

    @JvmStatic
    fun encodeToString(input: ByteArray, flags: Int): String =
        EncoderUtils.base64Encode(input, flags)

    @JvmStatic
    fun encodeToString(input: ByteArray, offset: Int, len: Int, flags: Int): String =
        encodeToString(slice(input, offset, len), flags)

    @JvmStatic
    fun encode(input: ByteArray, flags: Int): ByteArray =
        encodeToString(input, flags).toByteArray(Charsets.US_ASCII)

    @JvmStatic
    fun encode(input: ByteArray, offset: Int, len: Int, flags: Int): ByteArray =
        encode(slice(input, offset, len), flags)

    @JvmStatic
    fun decode(str: String, flags: Int): ByteArray =
        EncoderUtils.base64DecodeToByteArray(str, flags)

    @JvmStatic
    fun decode(input: ByteArray, flags: Int): ByteArray =
        decode(input.toString(Charsets.US_ASCII), flags)

    @JvmStatic
    fun decode(input: ByteArray, offset: Int, len: Int, flags: Int): ByteArray =
        decode(slice(input, offset, len), flags)

    private fun slice(input: ByteArray, offset: Int, len: Int): ByteArray {
        if (offset < 0 || len < 0 || offset > input.size - len) throw IndexOutOfBoundsException()
        return input.copyOfRange(offset, offset + len)
    }
}
