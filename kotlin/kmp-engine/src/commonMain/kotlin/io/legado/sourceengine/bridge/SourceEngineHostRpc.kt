package io.legado.sourceengine.bridge

import kotlinx.coroutines.runBlocking

/**
 * 同步 Provider/JS 绑定调用 Rust Host 的薄层。解析入口在整个操作期间固定当前 host，
 * 因此缓存、变量和 JS 库下载不会另起一条平台网络或文件访问路径。
 */
object SourceEngineHostRpc {
    fun storage(
        namespace: String,
        operation: String,
        key: String? = null,
        value: String? = null,
        ttlSeconds: Long? = null,
        keyPrefixes: List<String>? = null,
        contains: String? = null,
    ): String? = runBlocking {
        val storage = requireNotNull(SourceEngineHostRegistry.current()) {
            "Source-engine storage call has no active Rust host"
        }.storage
        when (operation) {
            "get" -> storage.get(namespace, requireNotNull(key))
            "put" -> {
                storage.put(namespace, requireNotNull(key), requireNotNull(value), ttlSeconds)
                null
            }
            "delete" -> {
                storage.delete(namespace, requireNotNull(key))
                null
            }
            "deletePrefixes" -> {
                storage.deletePrefixes(namespace, requireNotNull(keyPrefixes))
                null
            }
            "deleteContains" -> {
                storage.deleteContains(namespace, requireNotNull(contains))
                null
            }
            "clear" -> {
                storage.clear(namespace)
                null
            }
            else -> error("Unsupported source-engine storage operation: $operation")
        }
    }

    fun downloadText(url: String): String = runBlocking {
        val host = requireNotNull(SourceEngineHostRegistry.current()) {
            "Source-engine HTTP call has no active Rust host"
        }
        host.executeHttp(
            HostHttpRequest(
                url = url,
                headers = listOf(HostHeader("User-Agent", "Mozilla/5.0 LegadoSourceEngine")),
                maxRedirects = 20,
                acceptInvalidCerts = true,
            ),
        ).body.decodeToString()
    }
}
