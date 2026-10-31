package io.legado.app.utils

import io.legado.app.lib.icu4j.CharsetDetector

/** 各平台共用 ICU 频率表与识别算法。 */
internal fun detectCharsetName(bytes: ByteArray): String? = CharsetDetector().setText(bytes).detect()?.name
