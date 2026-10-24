package io.legado.app.help.http

import kotlin.concurrent.Volatile

interface CookieJarBridge {
    /** 对应 app 端 CookieManager.loadRequest */
    fun loadRequest(request: KmpRequest): KmpRequest
    /** 对应 app 端 CookieManager.saveResponse */
    fun saveResponse(response: KmpResponse)
}

object CookieJarBridgeHolder {
    @Volatile
    private var impl: CookieJarBridge? = null

    /** app 端启动早期注册一次(任何 ObsoleteUrlFactory 调用之前)。 */
    fun register(impl: CookieJarBridge) {
        this.impl = impl
    }

    /** 已注册则返回实现, 未注册返回 null (ObsoleteUrlFactory 拦截器无 cookie jar 时跳过)。 */
    fun get(): CookieJarBridge? = impl
}
