// 源平台来源：data/src/nativeMain/kotlin/io/legado/app/help/book/BookDisplayBridge.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.book

import io.legado.app.utils.ChineseUtils

/**
 * BookDisplayBridge actual (iOS / 鸿蒙)。
 */
actual fun chineseT2S(content: String): String = ChineseUtils.t2s(content)

actual fun chineseS2T(content: String): String = ChineseUtils.s2t(content)
