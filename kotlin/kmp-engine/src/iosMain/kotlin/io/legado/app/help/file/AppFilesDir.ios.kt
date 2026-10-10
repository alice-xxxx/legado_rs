// 源平台来源：data/src/iosMain/kotlin/io/legado/app/help/file/AppFilesDir.ios.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.file

import platform.Foundation.NSCachesDirectory
import platform.Foundation.NSDocumentDirectory
import platform.Foundation.NSApplicationSupportDirectory
import platform.Foundation.NSSearchPathForDirectoriesInDomains
import platform.Foundation.NSTemporaryDirectory
import platform.Foundation.NSUserDomainMask

/**
 * [AppFilesDir] 的 iOS 真实实现。
 *
 * KP3: 用 NSFileManager + NSSearchPathForDirectoriesInDomains 取真实沙盒路径:
 * - [filesDir]: `Documents/` (iTunes 文件共享可见, 持久化, 用户可读写)
 * - [cacheDir]: `Library/Caches/` (系统可清理, 不备份到 iCloud)
 * - [externalFilesDir]: iOS 无外部存储概念, 返回 null (与桌面端 jvm 一致)
 * - [externalCacheDir]: iOS 无外部存储概念, 返回 null (调用方回退到 [cacheDir])
 *
 * 路径解析参考 Apple 官方文档:
 * - https://developer.apple.com/documentation/foundation/filemanager
 * - https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/AccessingFilesandDirectories/AccessingFilesandDirectories.html
 *
 * 模式参考 [io.legado.app.help.config.AppConfigProviders] / `registerAndroidMediaNotificationProvider`。
 */
class IosAppFilesDir : AppFilesDir {

    /** Documents 目录 (沙盒内, 持久化, iTunes 文件共享可见)。 */
    override val filesDir: String = resolveSearchPath(NSDocumentDirectory)

    /** Library/Caches 目录 (沙盒内, 系统可清理, 不备份到 iCloud)。 */
    override val cacheDir: String = resolveSearchPath(NSCachesDirectory)

    /** iOS 无外部存储概念, 返回 null (与桌面端 jvm 一致)。 */
    override val externalFilesDir: String? = null

    /** iOS 无外部缓存概念, 返回 null (调用方回退到 [cacheDir])。 */
    override val externalCacheDir: String? = null

    /** 沙盒内持久封面缓存目录。 */
    override val coversDir: String
        get() = "$filesDir/covers"

}

/** 使用 Foundation 查询 iOS 沙盒中的标准目录。 */
private fun resolveSearchPath(directory: ULong): String {
    // NSSearchPathForDirectoriesInDomains 返回 NSArray<String>, 取首个匹配路径
    // iOS 沙盒保证 Documents/Caches 路径存在, 首个元素即绝对路径
    val paths = NSSearchPathForDirectoriesInDomains(
        directory = directory,
        domainMask = NSUserDomainMask,
        expandTilde = true
    )
    return paths.firstOrNull() as? String ?: run {
        // 路径查询失败时退化到沙盒临时目录, 避免空数组导致崩溃
        NSTemporaryDirectory()
    }
}

/**
 * iOS 宿主启动早期注册 [AppFilesDir] 的真实实现。
 *
 * 调用时机: iOS app 启动早期, 在任何 commonMain 代码调用 `AppFilesDirs.get()` 之前。
 *
 * 模式参考 `registerAndroidMediaNotificationProvider`。
 */
fun registerIosAppFilesDir() {
    AppFilesDirs.register(IosAppFilesDir())
}

/** Keeps generic iOS directories unchanged and scopes source-JS cache to the Rust private root. */
internal fun registerIosAppFilesDir(sourceEnginePrivateDir: String) {
    val applicationSupport = resolveSearchPath(NSApplicationSupportDirectory).trimEnd('/')
    val expectedRoot = "$applicationSupport/source-engine"
    check(sourceEnginePrivateDir.trimEnd('/') == expectedRoot) {
        "iOS source-engine data directory must be Library/Application Support/source-engine"
    }
    AppFilesDirs.register(IosAppFilesDir())
    io.legado.app.help.FileUtilsCommon.registerSourceEngineCacheDir(sourceEnginePrivateDir)
}
