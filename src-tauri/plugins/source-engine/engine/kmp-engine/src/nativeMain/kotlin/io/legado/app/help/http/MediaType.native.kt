package io.legado.app.help.http

actual fun KmpMediaType.charsetName(): String? {
    val value = this.toString()
    val idx = value.indexOf("charset=", ignoreCase = true)
    if (idx < 0) return null
    var charsetStr = value.substring(idx + 8).trim()
    // 截到分号或字符串结尾
    val end = charsetStr.indexOf(';')
    if (end > 0) {
        charsetStr = charsetStr.substring(0, end).trim()
    }
    // 去除可能的引号
    if (charsetStr.startsWith("\"") && charsetStr.endsWith("\"")) {
        charsetStr = charsetStr.substring(1, charsetStr.length - 1)
    }
    return charsetStr.takeIf { it.isNotEmpty() }
}
