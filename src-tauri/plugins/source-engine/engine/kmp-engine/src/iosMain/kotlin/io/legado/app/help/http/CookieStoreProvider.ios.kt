// 源平台来源：data/src/iosMain/kotlin/io/legado/app/help/http/CookieStoreProvider.ios.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.http

fun registerDefaultIosCookieStoreProvider() {
    CookieStoreProviders.register(SharedCookieStore)
}
