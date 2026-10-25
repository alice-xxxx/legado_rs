package io.legado.app.help.http

import kotlin.concurrent.Volatile

/** 源请求使用的 HTTP 模型客户端，由宿主注册传输实现。 */
interface OkHttpClientProvider {
    val okHttpClient: KmpHttpClient
}

object OkHttpClientProviders {
    @Volatile
    private var impl: OkHttpClientProvider? = null

    /** 宿主启动早期注册一次(任何 shared 调用之前)。 */
    fun register(impl: OkHttpClientProvider) {
        this.impl = impl
    }

    /** 取已注册实现；未注册抛出 IllegalStateException 帮助早期发现初始化遗漏。 */
    fun get(): OkHttpClientProvider =
        impl ?: error("宿主未注册源 HTTP 客户端")
}
