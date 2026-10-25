package io.legado.app.help.http

import io.legado.app.constant.AppLog
import io.legado.app.utils.NetworkUtils

/** 源 Cookie 的请求合并与响应保存；持久化通过源存储端口完成。 */
class SharedCookieJarBridge : CookieJarBridge {

    /**
     * 加载 Cookies 到请求中 (对照 app 端 CookieManager.loadRequest)。
     *
     * 取 url 所属二级域名 cookie, 与请求已有 Cookie 头合并; 设置失败时保留存储并返回原请求。
     */
    override fun loadRequest(request: KmpRequest): KmpRequest {
        val store = CookieStoreProviders.get() ?: return request
        val urlString = request.url.toString()
        val domain = NetworkUtils.getSubDomain(urlString)

        val storeCookie = store.getCookie(domain)
        val requestCookie = request.header("Cookie")

        val newCookie = mergeCookies(requestCookie, storeCookie) ?: return request

        return try {
            request.newBuilder()
                .removeHeader("Cookie")
                .addRequestHeader("Cookie", newCookie)
                .build()
        } catch (e: Exception) {
            AppLog.put("设置cookie出错，已保留登录数据 $domain", e)
            request
        }
    }

    /**
     * 从响应中保存 Cookies (对照 app 端 CookieManager.saveResponse → saveCookiesFromHeaders)。
     *
     * 解析 Set-Cookie 头, 按 persistent 分区:
     * - 会话期 cookie → [CookieStoreProvider.updateSessionCookie] (CacheManager 内存缓存);
     * - 持久 cookie → [CookieStoreProvider.replaceCookie] (DB + 内存 + WebView 同步)。
     */
    override fun saveResponse(response: KmpResponse) {
        val store = CookieStoreProviders.get() ?: return
        val url = response.request.url
        saveCookiesFromHeaders(url, response.headers(), store)
    }

    private fun saveCookiesFromHeaders(
        url: KmpHttpUrl,
        headers: KmpHeaders,
        store: CookieStoreProvider
    ) {
        val domain = NetworkUtils.getSubDomain(url.toString())
        val cookies = parseResponseCookies(url, headers)
        if (cookies.isEmpty()) return

        val (persistent, session) = cookies.partition { it.persistent }

        if (session.isNotEmpty()) {
            store.updateSessionCookie(domain, session.toCookieString())
        }

        if (persistent.isNotEmpty()) {
            store.replaceCookie(domain, persistent.toCookieString())
        }
    }
}

/**
 * 注册 [SharedCookieJarBridge] 到 [CookieJarBridgeHolder]。
 *
 * 宿主启动早期调用一次 (须在 [CookieStoreProviders] 注册之后)。
 * 替代各端独立 CookieJarBridge 实现 (desktop DesktopCookieJarBridge / app CookieManager 直接注册),
 * 统一桥接逻辑到 commonMain。
 */
fun registerSharedCookieJarBridge() {
    CookieJarBridgeHolder.register(SharedCookieJarBridge())
}
