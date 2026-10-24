// 源平台来源：data/src/iosMain/kotlin/org/jsoup/internal/HttpPlatform.ios.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package org.jsoup.internal

/**
 * jsoup 兼容层平台门面 iosMain actual。
 *
 * 与 jvm 的差异: [decompressBody] 原样返回 —— Ktor Darwin 层的 [KmpResponse] 在构造时已按
 * Content-Encoding 透明解压 (见 KmpHttpTypes.ios.kt decompressResponseBody) 且响应头未剥离,
 * 这里再解压会二次解压。
 */
internal actual fun decompressBody(bytes: ByteArray, contentEncoding: String?): ByteArray = bytes
