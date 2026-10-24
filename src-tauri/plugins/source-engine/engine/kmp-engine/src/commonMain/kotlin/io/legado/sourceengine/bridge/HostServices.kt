package io.legado.sourceengine.bridge

import io.legado.app.constant.AppConst
import io.legado.app.help.ExploreKindsCacheProvider
import io.legado.app.help.ExploreKindsCacheProviders
import io.legado.app.help.CacheManager
import io.legado.app.help.FileCacheProvider
import io.legado.app.help.FileCacheProviders
import io.legado.app.help.RuleBigDataProvider
import io.legado.app.help.RuleBigDataProviders
import io.legado.app.help.UserAgentProvider
import io.legado.app.help.UserAgentProviders
import io.legado.app.help.http.CookieStoreProviders
import io.legado.app.help.http.SharedCookieStore
import io.legado.app.help.http.registerSharedCookieJarBridge
import io.legado.app.help.source.SourceCacheProvider
import io.legado.app.help.source.SourceCacheProviders
import io.legado.app.help.source.SourceDebugLogger
import io.legado.app.help.source.SourceDebugLoggers
import io.legado.app.help.source.SourceNetworkProvider
import io.legado.app.help.source.SourceNetworkProviders
import okio.ByteString.Companion.decodeBase64
import okio.ByteString.Companion.toByteString

/** Shared engine services. Platform runtimes only install JS, crypto and native transports. */
internal fun installHostServices() {
    FileCacheProviders.impl = HostFileCache
    SourceCacheProviders.impl = HostSourceCache
    ExploreKindsCacheProviders.impl = HostExploreKindsCache
    RuleBigDataProviders.impl = HostRuleBigData
    SourceDebugLoggers.impl = HostSourceDebugLogger
    CookieStoreProviders.register(SharedCookieStore)
    registerSharedCookieJarBridge()
    SourceNetworkProviders.impl = HostSourceNetwork
    UserAgentProviders.impl = UserAgentProvider { AppConst.DEFAULT_USER_AGENT }
}

private object HostSourceCache : SourceCacheProvider {
    override fun get(key: String): String? = CacheManager.get(key)
    override fun put(key: String, value: String) = CacheManager.put(key, value)
    override fun put(key: String, value: String, saveTime: Int) = CacheManager.put(key, value, saveTime)
    override fun delete(key: String) = CacheManager.delete(key)
    override fun getFromMemory(key: String): Any? = CacheManager.getFromMemory(key)
    override fun putMemory(key: String, value: Any) = CacheManager.putMemory(key, value)
    override fun deleteMemory(key: String) = CacheManager.deleteMemory(key)
    override fun clearMemoryByPrefixes(prefixes: List<String>) = CacheManager.clearMemoryByPrefixes(prefixes)
    override fun asBinding(): Any = CacheManager
}

private object HostFileCache : FileCacheProvider {
    override fun put(key: String, value: String, saveTime: Int, persistent: Boolean) {
        EngineStorage.put(cacheNamespace(persistent), key, value, saveTime.toLong())
    }
    override fun getAsString(key: String, persistent: Boolean): String? =
        EngineStorage.get(cacheNamespace(persistent), key)
    override fun put(key: String, value: ByteArray, saveTime: Int, persistent: Boolean) {
        put(key, "base64:${value.toByteString().base64()}", saveTime, persistent)
    }
    override fun getAsBinary(key: String, persistent: Boolean): ByteArray? =
        getAsString(key, persistent)?.takeIf { it.startsWith("base64:") }
            ?.removePrefix("base64:")?.decodeBase64()?.toByteArray()
    override fun remove(key: String, persistent: Boolean) {
        EngineStorage.delete(cacheNamespace(persistent), key)
    }
    private fun cacheNamespace(persistent: Boolean) = if (persistent) "persistent-file-cache" else "file-cache"
}

private object HostExploreKindsCache : ExploreKindsCacheProvider {
    override fun getAsString(key: String): String? = EngineStorage.get("explore-kinds", key)
    override fun put(key: String, value: String) { EngineStorage.put("explore-kinds", key, value) }
    override fun remove(key: String) { EngineStorage.delete("explore-kinds", key) }
}

private object HostRuleBigData : RuleBigDataProvider {
    override fun putBookVariable(bookUrl: String, key: String, value: String?) = put(storageKey(bookUrl, key), value)
    override fun getBookVariable(bookUrl: String, key: String?): String? = get(storageKey(bookUrl, key.orEmpty()))
    override fun hasBookVariable(bookUrl: String, key: String): Boolean = get(storageKey(bookUrl, key)) != null
    override fun putChapterVariable(bookUrl: String, chapterUrl: String, key: String, value: String?) =
        put(storageKey(bookUrl, chapterUrl, key), value)
    override fun getChapterVariable(bookUrl: String, chapterUrl: String, key: String): String? =
        get(storageKey(bookUrl, chapterUrl, key))
    private fun put(key: String, value: String?) {
        if (value == null) EngineStorage.delete("book-variables", key) else EngineStorage.put("book-variables", key, value)
    }
    private fun get(key: String): String? = EngineStorage.get("book-variables", key)
    private fun storageKey(vararg parts: String) = parts.joinToString(":") {
        it.encodeToByteArray().toByteString().base64Url().trimEnd('=')
    }
}

private object HostSourceNetwork : SourceNetworkProvider {
    override fun getCookie(tag: String): String = SharedCookieStore.getCookie(tag)
    override fun replaceCookie(tag: String, cookie: String) = SharedCookieStore.replaceCookie(tag, cookie)
    override fun removeCookie(tag: String) = SharedCookieStore.removeCookie(tag)
    override fun asBinding(): Any = SharedCookieStore
}

private object HostSourceDebugLogger : SourceDebugLogger {
    override fun log(key: String, msg: String, print: Boolean, state: Int, showTime: Boolean) {
        if (print) println("[source:$state] $key ${msg.replace('\n', ' ').take(4096)}")
    }
    override fun log(msg: String) = log("", msg)
}
