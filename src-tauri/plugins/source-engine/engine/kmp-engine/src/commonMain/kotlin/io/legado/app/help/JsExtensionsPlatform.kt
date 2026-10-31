package io.legado.app.help

import io.legado.app.utils.Base64Lenient
import io.legado.app.utils.ChineseUtils
import io.legado.app.utils.formUrlEncode
import io.legado.app.utils.textCharsetCodec
import okio.ByteString.Companion.toByteString

/** Script helpers share all conversion logic; only system formatting and thread APIs vary. */
object JsExtensionsPlatform {
    fun urlEncode(str: String, charset: String): String = formUrlEncode(str, charset)
    fun strToBytes(str: String, charset: String): ByteArray = textCharsetCodec(charset).encode(str)
    fun bytesToStr(bytes: ByteArray, charset: String): String = textCharsetCodec(charset).decode(bytes)
    fun base64DecodeStr(str: String?, charset: String): String? = str?.let { bytesToStr(Base64Lenient.decode(it), charset) }
    fun chineseT2S(text: String): String = ChineseUtils.t2s(text)
    fun chineseS2T(text: String): String = ChineseUtils.s2t(text)
    fun sha256Hex(bytes: ByteArray): String = bytes.toByteString().sha256().hex()
    fun formatTimeUtc(time: Long, format: String, shiftHours: Int): String = JsPlatformRuntime.formatTimeUtc(time, format, shiftHours)
    fun isMainThread(): Boolean = JsPlatformRuntime.isMainThread()
    fun unsafeSslContext(): Any? = JsPlatformRuntime.unsafeSslContext()
}

internal expect object JsPlatformRuntime {
    fun formatTimeUtc(time: Long, format: String, shiftHours: Int): String
    fun isMainThread(): Boolean
    fun unsafeSslContext(): Any?
}
