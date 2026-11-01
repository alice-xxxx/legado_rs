package io.legado.app.utils

import com.fleeksoft.ksoup.Ksoup

/** Detect HTML or byte-array encodings using the shared charset implementation. */
object EncodingDetect {

    private val headTagRegex = "(?i)<head>[\\s\\S]*?</head>".toRegex()
    private val headOpenBytes = "<head>".encodeToByteArray()
    private val headCloseBytes = "</head>".encodeToByteArray()

    fun getHtmlEncode(bytes: ByteArray): String {
        try {
            var head: String? = null
            val startIndex = bytes.indexOf(headOpenBytes)
            if (startIndex > -1) {
                val endIndex = bytes.indexOf(headCloseBytes, startIndex)
                if (endIndex > -1) {
                    head = bytes.copyOfRange(startIndex, endIndex + headCloseBytes.size).decodeToString()
                }
            }
            val doc = Ksoup.parseBodyFragment(head ?: headTagRegex.find(bytes.decodeToString())!!.value)
            val metaTags = doc.getElementsByTag("meta")
            var charsetStr: String
            for (metaTag in metaTags) {
                charsetStr = metaTag.attr("charset")
                if (!charsetStr.isNullOrEmpty()) {
                    return charsetStr
                }
                val httpEquiv = metaTag.attr("http-equiv")
                if (httpEquiv.equals("content-type", true)) {
                    val content = metaTag.attr("content")
                    val idx = content.indexOf("charset=", ignoreCase = true)
                    charsetStr = if (idx > -1) {
                        content.substring(idx + "charset=".length)
                    } else {
                        content.substringAfter(";")
                    }
                    if (!charsetStr.isNullOrEmpty()) {
                        return charsetStr
                    }
                }
            }
        } catch (ignored: Exception) {
        }
        return getEncode(bytes)
    }

    fun getEncode(bytes: ByteArray): String {
        val match = detectCharsetName(bytes)
        return match ?: "UTF-8"
    }
}
