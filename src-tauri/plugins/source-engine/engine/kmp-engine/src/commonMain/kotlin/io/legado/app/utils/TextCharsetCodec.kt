package io.legado.app.utils

interface TextCharsetCodec {
    val name: String
    fun decode(bytes: ByteArray, offset: Int = 0, length: Int = bytes.size - offset): String
    fun encode(str: String): ByteArray
}

fun textCharsetCodec(name: String): TextCharsetCodec {
    val normalized = name.trim()
    return if (normalized.replace("-", "").replace("_", "").equals("GB18030", true)) gb18030Codec()
    else LibraryTextCharsetCodec(normalized)
}

// Fleeksoft 0.0.8 does not provide GB18030 on Native.
internal expect fun gb18030Codec(): TextCharsetCodec
