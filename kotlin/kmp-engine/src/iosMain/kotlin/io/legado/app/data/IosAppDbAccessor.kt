// 源平台来源：data/src/iosMain/kotlin/io/legado/app/data/IosAppDbAccessor.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.data

/**
 * iOS 宿主启动早期注册 [AppDbAccessor] 的入口 (委托 nativeMain 共用的 [NativeAppDbAccessor])。
 *
 * 前置依赖: [registerIosDatabaseDriver] 已注册 [AppDatabaseProviders]
 * (本文件通过 `AppDatabaseProviders.get().appDb` 取数据库实例)。
 *
 * 模式参考 desktop `Main.kt` 中 `AppDbProviders.register(DesktopAppDbAccessor())`。
 */
fun registerIosAppDbAccessor() {
    AppDbProviders.register(NativeAppDbAccessor())
}
