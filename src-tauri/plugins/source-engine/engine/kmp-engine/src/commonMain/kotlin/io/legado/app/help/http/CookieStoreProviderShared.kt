package io.legado.app.help.http

import io.legado.app.help.http.CookieStoreProviders.get
import kotlin.concurrent.Volatile

/** Host runtime installs SharedCookieStore before rule execution. */
interface CookieStoreProvider {

    /**
     * 保存 cookie (对应 app 端 `CookieStore.setCookie`)。
     *
     * @param url 完整 URL (必须 http/https 开头)
     * @param cookie cookie 字符串 ("k1=v1; k2=v2"), null 视为空串
     */
    fun setCookie(url: String, cookie: String?)

    /**
     * 替换/合并 cookie (对应 app 端 `CookieStore.replaceCookie`)。
     *
     * 已有 cookie 按 key 合并, 新 cookie 覆盖同名 key。
     *
     * @param url 完整 URL
     * @param cookie cookie 字符串
     */
    fun replaceCookie(url: String, cookie: String)

    /**
     * 获取 url 所属二级域名的 cookie (含 session cookie, 对应 app 端 `CookieStore.getCookie`)。
     *
     * @param url 完整 URL 或 domain
     * @return cookie 字符串 ("k1=v1; k2=v2"), 无 cookie 返回空串
     */
    fun getCookie(url: String): String

    /**
     * 获取 cookie 中指定 key 的值 (对应 app 端 `CookieStore.getKey`)。
     *
     * @param url 完整 URL 或 domain
     * @param key cookie key
     * @return cookie value, 不存在返回空串
     */
    fun getKey(url: String, key: String): String

    /**
     * 移除 url 所属二级域名的所有 cookie (对应 app 端 `CookieStore.removeCookie`)。
     *
     * @param url 完整 URL 或 domain
     */
    fun removeCookie(url: String)

    /** Remove a domain's cookies and propagate storage errors to explicit user actions. */
    fun removeCookieStrict(url: String)

    /**
     * 移除 url 所属二级域名中指定 key 的 cookie (对应 app 端 `CookieManager.removeCookie`)。
     *
     * @param url 完整 URL 或 domain
     * @param key cookie key
     */
    fun removeCookie(url: String, key: String)

    /**
     * 获取不含 session cookie 的持久 cookie (对应 app 端 `CookieManager.getCookieNoSession`)。
     *
     * @param url 完整 URL 或 domain
     * @return cookie 字符串, 无 cookie 返回空串
     */
    fun getCookieNoSession(url: String): String

    /**
     * 获取会话期 cookie (对应 app 端 `CookieManager.getSessionCookie`)。
     *
     * @param domain 二级域名
     * @return session cookie 字符串, 不存在返回 null
     */
    fun getSessionCookie(domain: String): String?

    /**
     * 更新会话期 cookie (对应 app 端 `CookieManager.updateSessionCookie`)。
     *
     * CookieJarBridge.saveResponse 解析响应 Set-Cookie 后, 会话期 cookie 经此方法写入
     * CacheManager 内存缓存 (`<domain>_session_cookie`), 与已有 session cookie 合并。
     *
     * @param domain 二级域名
     * @param cookies 会话期 cookie 字符串 ("k1=v1; k2=v2")
     */
    fun updateSessionCookie(domain: String, cookies: String)

    /**
     * 清除所有 cookie (对应 app 端 `CookieStore.clear`)。
     */
    fun clear()
}

/**
 * [CookieStoreProvider] 容器。宿主启动早期注册一次。
 *
 * shared 内访问点用 `CookieStoreProviders.get()?.setCookie(...)` 替代直接 app 单例调用,
 * 行为与 app 端一致, 仅多一层 provider 间接。
 * 未注册时 [get] 返回 null (调用方应处理 null, 与 [CookieJarBridgeHolder.get] 一致)。
 *
 * 模式参考 [io.legado.app.help.DirectLinkUploadStoreProviders] /
 * [CookieJarBridgeHolder]。
 */
object CookieStoreProviders {

    @Volatile
    private var impl: CookieStoreProvider? = null

    /** 宿主启动早期注册一次 (任何 shared 调用之前)。 */
    fun register(impl: CookieStoreProvider) {
        this.impl = impl
    }

    /** 获取已注册实现, 未注册返回 null。 */
    fun get(): CookieStoreProvider? = impl
}
