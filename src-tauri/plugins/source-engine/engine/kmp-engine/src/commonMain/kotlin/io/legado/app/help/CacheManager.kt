package io.legado.app.help

import com.script.jsdispatch.JsApi
import io.legado.app.help.source.SourceStorageProviders
import io.legado.app.help.source.SourceStorageNamespace
import io.legado.app.model.analyzeRule.QueryTTF
import io.legado.app.utils.systemCurrentTimeMillis

private val memoryLruCache = CommonLruCache<String, Any>(1024 * 1024 * 50) { _, value ->
    estimateMemorySize(value)
}

private val queryTTFMap = CommonLruCache<String, QueryTTF>(4)

private fun estimateMemorySize(value: Any?): Int {
    return when (value) {
        null -> 0
        is CharSequence -> 40 + 2 * value.length
        is Number, is Boolean, is Char -> 24
        is ByteArray -> 16 + value.size
        is Map<*, *> -> 40 + value.size * 40
        is Collection<*> -> 40 + value.size * 40
        else -> 40 + 2 * value.toString().length
    }
}

object AppCacheManager {

    fun put(key: String, queryTTF: QueryTTF) {
        queryTTFMap.put(key, queryTTF)
    }

    fun getQueryTTF(key: String): QueryTTF? {
        return queryTTFMap[key]
    }

    fun clearSourceVariables() {
        // 遍历 snapshot 副本, 删除前缀匹配的内存缓存 (与原 app 端行为一致)。
        memoryLruCache.snapshot().keys.forEach {
            if (it.startsWith("v_")
                || it.startsWith("userInfo_")
                || it.startsWith("loginHeader_")
                || it.startsWith("sourceVariable_")
            ) {
                memoryLruCache.remove(it)
            }
        }
    }

}

@JsApi
@Suppress("unused")
object CacheManager {

    // 短参显式重载: 补回原 @JvmOverloads 生成的 JVM 签名 (commonMain 无该注解), 书源 JS 按 arity 匹配
    fun put(key: String, value: Any) = put(key, value, 0)

    fun put(key: String, value: Any, saveTime: Int = 0) {
        val deadline =
            if (saveTime == 0) 0L else systemCurrentTimeMillis() + saveTime * 1000L
        when (value) {
            is ByteArray -> FileCacheProviders.get().put(key, value, saveTime)
            else -> {
                SourceStorageProviders.get().put(SourceStorageNamespace.Cache, key, "$deadline\n$value", saveTime.toLong())
            }
        }
    }

    fun putMemory(key: String, value: Any) {
        memoryLruCache.put(key, value)
    }

    //从内存中获取数据 使用lruCache
    fun getFromMemory(key: String): Any? {
        return memoryLruCache[key]
    }

    fun clearMemoryByPrefixes(prefixes: List<String>) {
        memoryLruCache.snapshot().keys.filter { key -> prefixes.any(key::startsWith) }
            .forEach(memoryLruCache::remove)
    }

    fun deleteMemory(key: String) {
        memoryLruCache.remove(key)
    }

    fun get(key: String): String? {
        val encoded = SourceStorageProviders.get().get(SourceStorageNamespace.Cache, key) ?: return null
        val separator = encoded.indexOf('\n')
        check(separator > 0) { "Invalid source cache entry" }
        val deadline = encoded.substring(0, separator).toLong()
        if (deadline == 0L || deadline > systemCurrentTimeMillis()) {
            return encoded.substring(separator + 1)
        }
        return null
    }

    fun getInt(key: String): Int? {
        return get(key)?.toIntOrNull()
    }

    fun getLong(key: String): Long? {
        return get(key)?.toLongOrNull()
    }

    fun getDouble(key: String): Double? {
        return get(key)?.toDoubleOrNull()
    }

    fun getFloat(key: String): Float? {
        return get(key)?.toFloatOrNull()
    }

    fun getByteArray(key: String): ByteArray? {
        return FileCacheProviders.get().getAsBinary(key)
    }

    fun putFile(key: String, value: String) = putFile(key, value, 0)

    fun putFile(key: String, value: String, saveTime: Int = 0) {
        FileCacheProviders.get().put(key, value, saveTime)
    }

    fun getFile(key: String): String? {
        return FileCacheProviders.get().getAsString(key)
    }

    fun delete(key: String) {
        SourceStorageProviders.get().delete(SourceStorageNamespace.Cache, key)
        deleteMemory(key)
        FileCacheProviders.get().remove(key)
    }
}
