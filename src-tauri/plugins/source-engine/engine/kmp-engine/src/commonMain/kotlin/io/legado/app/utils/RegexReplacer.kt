package io.legado.app.utils

import kotlin.concurrent.Volatile

interface RegexReplacer {
    /**
     * 带超时检测的正则替换 (对应 app 端 CharSequence.replace 扩展)。
     *
     * @param source     待替换文本
     * @param regex      正则
     * @param replacement 替换串 (app 端实现支持 @js: 前缀动态替换)
     * @param timeout    超时毫秒
     */
    fun replace(
        source: CharSequence,
        regex: Regex,
        replacement: String,
        timeout: Long
    ): String
}

/**
 * RegexReplacer provider 容器。宿主启动早期注册一次。
 *
 * shared 内访问点用 `RegexReplacers.get().replace(...)` 替代原
 * `source.replace(regex, replacement, timeout)`, 行为完全一致,
 * 仅多一层 provider 间接。
 *
 * Android 端经 [io.legado.app.model.webBook.registerAndroidWebBookProviders]
 * 各平台注册 [DeadlineRegexReplacer]，共享替换与超时处理。
 */
object RegexReplacers {
    @Volatile
    private var impl: RegexReplacer? = null

    /** 宿主启动早期注册一次(任何 webBook 调用之前)。 */
    fun register(impl: RegexReplacer) {
        this.impl = impl
    }

    /** 获取已注册实现, 未注册抛出 IllegalStateException。 */
    fun get(): RegexReplacer = impl ?: error("RegexReplacers not registered")
}
