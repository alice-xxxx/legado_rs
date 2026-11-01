// 源平台来源：foundation/src/nativeMain/kotlin/io/legado/app/utils/JvmPlatformTypes.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.utils

actual class URL actual constructor(val url: String) {
    actual override fun toString(): String = url

    actual fun toExternalForm(): String = url

}

actual fun URL.urlQuery(): String? =
    url.substringAfter('?', "").substringBefore('#').takeIf { it.isNotEmpty() }
