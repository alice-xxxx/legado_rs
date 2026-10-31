package io.legado.app.utils

private fun formSafe(char: Char): Boolean = char in 'a'..'z' || char in 'A'..'Z' || char in '0'..'9' || char in "*-._"
private const val FORM_HEX = "0123456789ABCDEF"

/** Java URLEncoder 语义：空格转 +，连续的非安全字符按指定字符集一起编码。 */
fun formUrlEncode(value: String, charset: String = "UTF-8"): String {
    val codec = textCharsetCodec(charset)
    return buildString(value.length) {
        var index = 0
        while (index < value.length) {
            val char = value[index]
            when {
                formSafe(char) -> { append(char); index++ }
                char == ' ' -> { append('+'); index++ }
                else -> {
                    var end = index + 1
                    while (end < value.length && !formSafe(value[end]) && value[end] != ' ') end++
                    for (byte in codec.encode(value.substring(index, end))) {
                        val unsigned = byte.toInt() and 255
                        append('%').append(FORM_HEX[unsigned ushr 4]).append(FORM_HEX[unsigned and 15])
                    }
                    index = end
                }
            }
        }
    }
}

fun formUrlDecode(value: String, charset: String = "UTF-8"): String {
    val codec = textCharsetCodec(charset)
    return buildString(value.length) {
        var index = 0
        while (index < value.length) {
            when (val char = value[index]) {
                '+' -> { append(' '); index++ }
                '%' -> {
                    val bytes = ByteArray((value.length - index) / 3)
                    var count = 0
                    while (index < value.length && value[index] == '%') {
                        require(index + 2 < value.length) { "Incomplete URL escape" }
                        val high = value[index + 1].digitToIntOrNull(16)
                        val low = value[index + 2].digitToIntOrNull(16)
                        require(high != null && low != null) { "Invalid URL escape" }
                        bytes[count++] = ((high shl 4) or low).toByte()
                        index += 3
                    }
                    append(codec.decode(bytes, 0, count))
                }
                else -> { append(char); index++ }
            }
        }
    }
}
