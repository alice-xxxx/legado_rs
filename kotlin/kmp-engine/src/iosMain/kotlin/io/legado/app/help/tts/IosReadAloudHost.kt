// 源平台来源：data/src/iosMain/kotlin/io/legado/app/help/tts/IosReadAloudHost.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.tts

/**
 * iOS 端朗读宿主: 实现全在 [ReadAloudHostShared], 无 iOS 专属部分。
 *
 * 播控卡片 (NowPlayingInfoCenter) 由 `IosMediaNotificationController` 注册本对象接管。
 */
object IosReadAloudHost : ReadAloudHostShared()
