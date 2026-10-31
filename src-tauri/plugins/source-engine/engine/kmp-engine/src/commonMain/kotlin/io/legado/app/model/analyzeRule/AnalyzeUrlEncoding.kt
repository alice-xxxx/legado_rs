package io.legado.app.model.analyzeRule

import io.legado.app.utils.EncoderUtils
import io.legado.app.utils.NetworkUtils
import io.legado.app.utils.formUrlEncode
import io.legado.app.utils.textCharsetCodec

/** 查询串和表单参数共用同一编码策略，特殊字符与旧 escape 模式保持分开。 */
internal fun encodeUrlParams(params: String, charset: String?, isQuery: Boolean): String {
    val checkEncoded = charset.isNullOrEmpty()
    val escape = charset == "escape"
    val name = charset?.takeIf { it.isNotEmpty() } ?: "UTF-8"
    if (isQuery && !escape) {
        return if (NetworkUtils.encodedQuery(params)) params
        else urlQueryEncoder.encode(params, textCharsetCodec(name)::encode)
    }
    fun encode(value: String): String = when {
        checkEncoded && NetworkUtils.encodedForm(value) -> value
        escape -> EncoderUtils.escape(value)
        else -> formUrlEncode(value, name)
    }
    return params.removeSuffix("&").split('&').dropWhile { it.isEmpty() }.joinToString("&") { pair ->
        val equal = pair.indexOf('=')
        if (equal < 0) encode(pair) else encode(pair.substring(0, equal)) + "=" + encode(pair.substring(equal + 1))
    }
}
