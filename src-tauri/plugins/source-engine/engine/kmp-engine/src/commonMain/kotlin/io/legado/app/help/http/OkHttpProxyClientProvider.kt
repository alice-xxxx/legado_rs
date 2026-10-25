package io.legado.app.help.http

import kotlin.concurrent.Volatile

/** 按源配置取得代理客户端；实际网络和代理能力归宿主。 */
interface OkHttpProxyClientProvider {
    fun getProxyClient(proxy: String?): KmpHttpClient
}

object OkHttpProxyClientProviders {
    @Volatile
    var impl: OkHttpProxyClientProvider? = null

    /** 取已注册实现；未注册抛出 IllegalStateException 帮助早期发现初始化遗漏。 */
    fun get(): OkHttpProxyClientProvider =
        impl ?: error("宿主未注册源 HTTP 客户端")
}
