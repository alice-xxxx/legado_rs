package io.legado.sourceengine.bridge

import io.legado.sourceengine.android.rustSourceHttpRequest
import io.legado.sourceengine.android.rustSourceStorageRequest
import io.legado.sourceengine.android.AndroidSourceEngineRuntime
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Android 宿主适配器：应用提供私有数据目录，每次存储请求都显式携带，避免并发书源操作
 * 争用 Rust 进程级路径。网络和存储实际由同一 Rust crate 执行。
 */
class AndroidRustSourceEngineHost(appDataDirectory: String) : SourceEngineHost {
    private val dataDirectory = appDataDirectory

    override val storage: HostStorage = object : HostStorage {
        override suspend fun get(namespace: String, key: String): String? = storageCall(
            SourceEngineHostWire.encodeStorageRequest(namespace, "get", key = key),
        )

        override suspend fun put(namespace: String, key: String, value: String, ttlSeconds: Long?) {
            storageCall(
                SourceEngineHostWire.encodeStorageRequest(
                    namespace, "put", key = key, value = value, ttlSeconds = ttlSeconds,
                ),
            )
        }

        override suspend fun delete(namespace: String, key: String) {
            storageCall(SourceEngineHostWire.encodeStorageRequest(namespace, "delete", key = key))
        }

        override suspend fun deletePrefixes(namespace: String, prefixes: List<String>) {
            storageCall(
                SourceEngineHostWire.encodeStorageRequest(
                    namespace, "deletePrefixes", keyPrefixes = prefixes,
                ),
            )
        }

        override suspend fun deleteContains(namespace: String, value: String) {
            storageCall(
                SourceEngineHostWire.encodeStorageRequest(namespace, "deleteContains", contains = value),
            )
        }

        override suspend fun clear(namespace: String) {
            storageCall(SourceEngineHostWire.encodeStorageRequest(namespace, "clear"))
        }
    }

    override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse = withContext(Dispatchers.IO) {
        SourceEngineHostWire.decodeHttpResponse(
            rustSourceHttpRequest(SourceEngineHostWire.encodeHttpRequest(request)),
            request.url,
        )
    }

    private suspend fun storageCall(requestJson: String): String? = withContext(Dispatchers.IO) {
        SourceEngineHostWire.decodeStorageResponse(
            rustSourceStorageRequest(requestJson, dataDirectory),
        )
    }
}

/** Android 书源解析入口，由应用直接调用。 */
object AndroidSourceEngine {
    suspend fun execute(call: SourceEngineCall, appDataDirectory: String): String {
        AndroidRustSourceHttpProvider.install()
        AndroidSourceEngineRuntime.install()
        return WebBookSourceRuleExecutor.execute(call, AndroidRustSourceEngineHost(appDataDirectory))
    }

    /** Native Tauri commands pass the same JSON wire request used by the desktop Rust command. */
    suspend fun executeJson(requestJson: String, appDataDirectory: String): String =
        execute(decodeSourceEngineCall(requestJson), appDataDirectory)
}

/** Java Tauri plugins have no suspend ABI; execute on their background executor via runBlocking. */
object AndroidSourceEngineBlocking {
    @JvmStatic
    fun executeJson(requestJson: String, appDataDirectory: String): String =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.executeJson(requestJson, appDataDirectory) }
}
