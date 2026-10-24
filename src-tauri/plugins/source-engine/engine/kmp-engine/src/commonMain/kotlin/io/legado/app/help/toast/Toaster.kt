package io.legado.app.help.toast

import kotlin.concurrent.Volatile

interface Toaster {

    /** 短时间显示 [message]。 */
    fun toast(message: String)

    /** 长时间显示 [message]。 */
    fun toastLong(message: String)
}

/**
 * [Toaster] 容器 (provider 注入模式)。
 *
 * 宿主启动早期注册一次 (App.onCreate / desktop main), shared 内通过 [get] 获取。
 * 未注册时调用 [get] 抛 [IllegalStateException], 帮助早期发现初始化遗漏。
 *
 * 安卓端调用 `registerAndroidToaster(appCtx)` (见 androidMain),
 * 桌面端调用 `registerDesktopToaster()` (见 jvmMain)。
 */
object Toasters {

    @Volatile
    private var impl: Toaster? = null

    /** 宿主启动早期注册一次 (任何 toast 调用之前)。 */
    fun register(impl: Toaster?) {
        this.impl = impl
    }

    fun current(): Toaster? = impl

    /** 获取已注册实现, 未注册抛出 IllegalStateException。 */
    fun get(): Toaster =
        impl ?: error("Toasters not registered; call registerAndroidToaster() or registerDesktopToaster() first")

    /** 仅测试场景: 清空注册 (生产代码勿调用)。 */
    fun reset() {
        impl = null
    }
}
