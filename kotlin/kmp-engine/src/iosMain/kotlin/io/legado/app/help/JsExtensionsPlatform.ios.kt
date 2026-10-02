// 源平台来源：data/src/iosMain/kotlin/io/legado/app/help/JsExtensionsPlatform.ios.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help

import platform.Foundation.NSThread

/**
 * iOS 主线程判定: `NSThread.isMainThread` (与 app 端 `Looper.getMainLooper().thread ==
 * currentThread` 语义一致, JS 引擎线程/网络线程返回 false, 主线程返回 true)。
 */
internal actual fun isMainThreadPlatform(): Boolean = NSThread.isMainThread
