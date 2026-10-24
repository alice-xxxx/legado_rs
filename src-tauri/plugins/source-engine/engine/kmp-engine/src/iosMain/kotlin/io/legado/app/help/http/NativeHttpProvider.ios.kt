// 源平台来源：data/src/iosMain/kotlin/io/legado/app/help/http/NativeHttpProvider.ios.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.http

/**
 * iOS actual: 使用 Ktor Darwin engine 构造客户端 (见 [KmpHttpClientBuilder.build])。
 */
internal actual fun buildNativeProxyClient(
    host: String,
    port: Int,
    username: String?,
    password: String?,
): KmpHttpClient = KmpHttpClientBuilder()
    .proxy(host, port, username, password)
    .build()
