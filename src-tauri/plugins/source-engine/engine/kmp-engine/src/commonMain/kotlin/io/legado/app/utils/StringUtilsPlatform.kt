package io.legado.app.utils

import okio.Buffer
import okio.GzipSink
import okio.buffer
import okio.use

/**
 * StringUtils 平台相关 expect 门面。
 *
 * - [createWordCountFormatter]: 返回一个 (Double) -> String 的格式化闭包,
 *   actual 在 jvmAndAndroidMain 持有 [java.text.DecimalFormat]("#.#") (HALF_EVEN + 去末尾 0);
 *   commonMain 通过 lazy 缓存闭包, 保留原 "by lazy { DecimalFormat(...) }" 单例语义。
 *
 * gzip 压缩由公共 Okio 实现，关闭压缩流后再编码完整结果。
 */
internal expect fun createWordCountFormatter(): (Double) -> String

internal fun gzipAndBase64Encode(str: String): String {
    val bytes = Buffer()
    GzipSink(bytes).buffer().use { it.writeUtf8(str) }
    return Base64Lenient.encodeToString(bytes.readByteArray())
}
