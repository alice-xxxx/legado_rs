package io.legado.app.model.script

import io.legado.app.data.entities.BaseSource
import io.legado.app.help.http.OkHttpClientProviders
import io.legado.app.help.http.newCallStrResponse
import io.legado.app.utils.KS_JSON
import io.legado.app.utils.MD5Utils
import io.legado.app.utils.isAbsUrl
import io.legado.app.utils.isJsonObject
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.decodeFromString
import kotlin.coroutines.CoroutineContext

/** 三端共用脚本库下载、编译和版本失效；执行上下文仍由线程独占。 */
object SharedLibraryScopes : SharedJsScopeProvider {
    internal class Entry(val scope: JsScope, val version: Long)
    private class Library(val scripts: List<JsCompiledScript>, val version: Long)
    private val lock = SynchronizedObject()
    private val libraries = mutableMapOf<String, Library>()
    private val downloads = mutableMapOf<String, String>()
    private var version = 0L

    override fun getScope(jsLib: String?, enableDangerousApi: Boolean, coroutineContext: CoroutineContext?): JsScope? =
        scope(jsLib, enableDangerousApi, coroutineContext, null)

    override fun getScope(jsLib: String?, enableDangerousApi: Boolean, coroutineContext: CoroutineContext?, bindings: JsBindings): JsScope? =
        scope(jsLib, enableDangerousApi, coroutineContext, bindings)

    private fun scope(jsLib: String?, dangerousApi: Boolean, context: CoroutineContext?, bindings: JsBindings?): JsScope? {
        if (jsLib.isNullOrBlank()) return null
        val engine = JsEngines.get()
        val cache = sharedLibraryThreadCache()
        cache.flush()
        val key = MD5Utils.md5Encode(jsLib)
        val library = library(key, jsLib, engine)
        val scopeKey = "$key:$dangerousApi:${(bindings?.get("source") as? BaseSource)?.getKey().orEmpty()}"
        cache.get(scopeKey)?.takeIf { it.version == library.version }?.let { return it.scope }
        val scope = engine.getRuntimeScope(JsBindings().apply { this.dangerousApi = dangerousApi })
        try {
            val keys = bindings?.let { engine.injectBindings(scope, it) }
            try {
                library.scripts.forEach { it.eval(scope, context) }
            } finally {
                if (keys != null) engine.cleanupBindings(scope, keys)
            }
        } catch (error: Throwable) {
            scope.close()
            throw error
        }
        cache.put(scopeKey, Entry(scope, library.version))
        return scope
    }

    private fun library(key: String, jsLib: String, engine: JsEngine): Library {
        synchronized(lock) { libraries[key]?.let { return it } }
        val sources = if (jsLib.isJsonObject()) {
            KS_JSON.decodeFromString<Map<String, String>>(jsLib).values.filter { it.isAbsUrl() }.map { url ->
                synchronized(lock) { downloads[url] } ?: runBlocking {
                    OkHttpClientProviders.get().okHttpClient.newCallStrResponse { url(url) }.body
                }?.also { synchronized(lock) { downloads[url] = it } }
                ?: error("脚本库下载失败：$url")
            }
        } else listOf(jsLib)
        val scripts = sources.map(engine::compile)
        return synchronized(lock) { libraries.getOrPut(key) { Library(scripts, ++version) } }
    }

    override fun remove(jsLib: String?) {
        if (jsLib.isNullOrBlank()) return
        synchronized(lock) { libraries.remove(MD5Utils.md5Encode(jsLib)) }
        sharedLibraryThreadCache().flush()
    }

    override fun clearAll() {
        synchronized(lock) { libraries.clear() }
        sharedLibraryThreadCache().clear()
    }
}

/** 只在本线程 JS 执行栈为空时释放淘汰项，避免重入回调访问已释放的上下文。 */
internal class SharedLibraryThreadCache {
    private val entries = LinkedHashMap<String, SharedLibraryScopes.Entry>()
    private val pending = mutableListOf<JsScope>()

    fun get(key: String): SharedLibraryScopes.Entry? = entries.remove(key)?.also { entries[key] = it }

    fun put(key: String, entry: SharedLibraryScopes.Entry) {
        entries.put(key, entry)?.let { pending.add(it.scope) }
        if (entries.size > 4) entries.remove(entries.keys.first())?.let { pending.add(it.scope) }
        flush()
    }

    fun clear() {
        pending.addAll(entries.values.map { it.scope })
        entries.clear()
        flush()
    }

    fun flush() {
        if (JsEngines.get().currentContext() != null) return
        val retired = pending.toList()
        pending.clear()
        retired.forEach { it.close() }
    }
}

internal expect fun sharedLibraryThreadCache(): SharedLibraryThreadCache
