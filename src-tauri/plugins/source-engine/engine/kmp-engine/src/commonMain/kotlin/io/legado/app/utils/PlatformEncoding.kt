package io.legado.app.utils

/**
 * Data URI 使用公共宽松 Base64 解码，忽略杂字符并接受 URL-safe 字母表。
 */
object MimeBase64Decoder {
    fun decode(input: String): ByteArray = Base64Lenient.decode(input)
}
