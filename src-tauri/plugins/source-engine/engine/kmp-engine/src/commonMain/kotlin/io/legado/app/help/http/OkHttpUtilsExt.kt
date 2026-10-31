package io.legado.app.help.http

import io.legado.app.utils.EncodingDetect
import io.legado.app.utils.Utf8BomUtils
import io.legado.app.utils.textCharsetCodec

suspend fun KmpHttpClient.newCallStrResponse(
    retry: Int = 0,
    builder: KmpRequestBuilder.() -> Unit
): StrResponse = newCallResponse(retry, builder).let { StrResponse(it, it.body.text()) }

fun KmpResponseBody.text(encode: String? = null): String {
    val bytes = Utf8BomUtils.removeUTF8BOM(bytes())
    if (encode != null) return textCharsetCodec(encode).decode(bytes)
    contentType()?.charsetName()?.let { charset ->
        runCatching { textCharsetCodec(charset).decode(bytes) }.getOrNull()?.let { return it }
    }
    return runCatching { textCharsetCodec(EncodingDetect.getHtmlEncode(bytes)).decode(bytes) }
        .getOrElse { bytes.decodeToString() }
}

expect fun KmpResponseBody.decompressed(): KmpResponseBody
expect fun KmpRequestBuilder.postMultipart(type: String?, form: Map<String, Any>)
