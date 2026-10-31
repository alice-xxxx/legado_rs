package io.legado.app.utils

internal actual fun gb18030Codec(): TextCharsetCodec = Gb18030Codec

private object Gb18030Codec : TextCharsetCodec {
    override val name: String = "GB18030"
    override fun decode(bytes: ByteArray, offset: Int, length: Int): String {
        require(offset >= 0 && length >= 0 && offset <= bytes.size - length)
        return if (length == 0) "" else platformDecodeGb18030(bytes, offset, length)
    }
    override fun encode(str: String): ByteArray =
        if (str.isEmpty()) ByteArray(0) else platformEncodeGb18030(str)
}

internal expect fun platformDecodeGb18030(bytes: ByteArray, offset: Int, length: Int): String
internal expect fun platformEncodeGb18030(str: String): ByteArray
