package io.legado.app.help.http

import com.script.jsdispatch.JsApi
import io.legado.sourceengine.bridge.EngineStorage
import io.legado.app.help.CacheManager
import io.legado.app.help.http.SharedCookieStore.getCookieNoSession
import io.legado.app.help.http.SharedCookieStore.getSessionCookie
import io.legado.app.help.http.SharedCookieStore.removeCookie
import io.legado.app.help.http.SharedCookieStore.updateSessionCookie
import io.legado.app.utils.NetworkUtils

@JsApi
@Suppress("unused")
object SharedCookieStore : CookieStoreBase(), CookieStoreProvider {

    override fun onInsertCookieToDb(domain: String, cookieStr: String) {
        EngineStorage.put("cookies", domain, cookieStr)
    }

    override fun onDeleteCookieFromDb(domain: String) {
        EngineStorage.delete("cookies", domain)
    }

    override fun onClearOkHttpCookie() {
        EngineStorage.deleteContains("cookies", "|")
    }

    // ===== CookieManager 语义 (对照 app 端 CookieManager 同名方法) =====

    public override fun getCookieNoSession(url: String): String {
        val domain = NetworkUtils.getSubDomain(url)
        val cacheCookie = CacheManager.getFromMemory("${domain}_cookie") as? String
        return cacheCookie
            ?: EngineStorage.get("cookies", domain)
            ?: ""
    }

    public override fun getSessionCookie(domain: String): String? {
        return CacheManager.getFromMemory("${domain}_session_cookie") as? String
    }

    public override fun removeCookie(url: String, key: String) {
        val domain = NetworkUtils.getSubDomain(url)

        getSessionCookie(domain)?.let { cookieToMap(it) }?.let { map ->
            if (map.remove(key) != null) {
                mapToCookie(map)?.let {
                    CacheManager.putMemory("${domain}_session_cookie", it)
                }
            }
        }

        val cookie = getCookieNoSession(url)
        if (cookie.isNotEmpty()) {
            val cookieMap = cookieToMap(cookie)
            if (cookieMap.remove(key) != null) {
                setCookie(url, mapToCookie(cookieMap))
            }
        }
    }

    override fun updateSessionCookie(domain: String, cookies: String) {
        val cacheKey = "${domain}_session_cookie"
        val sessionCookie = CacheManager.getFromMemory(cacheKey) as? String
        val ck =
            if (sessionCookie.isNullOrEmpty()) cookies else mergeCookies(sessionCookie, cookies)
        ck?.let {
            CacheManager.putMemory(cacheKey, it)
        }
    }
}
