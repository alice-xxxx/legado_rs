package io.legado.app.help.file

import io.legado.app.help.file.AppFilesDirs.get
import kotlin.concurrent.Volatile

interface AppFilesDir {

    /** 应用内部文件目录 (对应 Android `Context.filesDir`)。 */
    val filesDir: String

    /** 应用内部缓存目录 (对应 Android `Context.cacheDir`)。 */
    val cacheDir: String

    /** 应用外部文件目录 (对应 Android `Context.getExternalFilesDir(null)`), 无则 null。 */
    val externalFilesDir: String?

    /**
     * 应用外部缓存目录 (对应 Android `Context.externalCacheDir`), 无则 null。
     *
     * 桌面端无外部缓存概念, 返回 null; 调用方需回退到 [cacheDir]
     * (参考 [io.legado.app.help.storage.BackupShared] 的 externalFilesDir ?: filesDir 模式)。
     */
    val externalCacheDir: String?

    /**
     * 本地书与遗留封面物理目录 (本地书封面落盘处, `FileBook.getCoverPath` 派生), 无则 null。
     *
     * 备份/恢复映射为独立的 `oldCovers/` 命名空间，避免与 `customImg/covers`
     * 混用；图片路径解析也用它解析 `oldCovers/<name>` (及历史 `coverCache/<name>`) 相对引用。
     */
    val coversDir: String? get() = null
}

/**
 * [AppFilesDir] 容器 (provider 注入模式)。
 *
 * 宿主启动早期注册一次 (App.onCreate / desktop main), shared 内通过 [get] 获取。
 * 未注册时调用 [get] 抛 [IllegalStateException]。
 *
 * 安卓端调用 `registerAndroidAppFilesDir(context)` (见 androidMain),
 * 桌面端调用 `registerDesktopAppFilesDir()` (见 jvmMain)。
 */
object AppFilesDirs {

    @Volatile
    private var impl: AppFilesDir? = null

    /** 宿主启动早期注册一次 (任何目录访问之前)。 */
    fun register(impl: AppFilesDir) {
        this.impl = impl
    }

    /** 获取已注册实现, 未注册抛出 IllegalStateException。 */
    fun get(): AppFilesDir =
        impl ?: error("AppFilesDirs not registered; call registerAndroidAppFilesDir() or registerDesktopAppFilesDir() first")

    /** 仅测试场景: 清空注册 (生产代码勿调用)。 */
    fun reset() {
        impl = null
    }
}
