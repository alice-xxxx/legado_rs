// 源平台来源：foundation/src/nativeMain/kotlin/io/legado/app/exception/SecurityException.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.exception

/**
 * Kotlin/Native 无 java.lang.SecurityException, 补一个同名类型供 iOS/鸿蒙侧路径校验抛出。
 */
class SecurityException(message: String) : Exception(message)
