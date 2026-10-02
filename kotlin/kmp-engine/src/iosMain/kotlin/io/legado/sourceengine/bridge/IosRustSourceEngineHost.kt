@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.sourceengine.bridge

import io.legado.sourceengine.rust.legado_source_host_http
import io.legado.sourceengine.rust.legado_source_host_storage
import io.legado.sourceengine.rust.legado_source_host_string_free
import kotlinx.cinterop.ByteVar
import kotlinx.cinterop.CPointer
import kotlinx.cinterop.toKString
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * iOS 通过 Rust C ABI 提交请求。Swift 提供应用私有数据目录；HTTP、Cookie、存储命名空间
 * 校验和文件读写均由 Rust 执行。
 */
class IosRustSourceEngineHost(appDataDirectory: String) : SourceEngineHost {
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

    override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse = withContext(Dispatchers.Default) {
        SourceEngineHostWire.decodeHttpResponse(
            rustResponse(requestJson = SourceEngineHostWire.encodeHttpRequest(request)) { input ->
                legado_source_host_http(input)
            },
            request.url,
        )
    }

    private suspend fun storageCall(requestJson: String): String? = withContext(Dispatchers.Default) {
        SourceEngineHostWire.decodeStorageResponse(
            rustResponse(requestJson) { request ->
                legado_source_host_storage(request, dataDirectory)
            },
        )
    }

    private inline fun rustResponse(
        requestJson: String,
        call: (String) -> CPointer<ByteVar>?,
    ): String {
        // cinterop maps const char* inputs to Kotlin String; returned char* stays a raw pointer
        // because the Rust-owned allocation must be copied and explicitly freed below.
        val response = call(requestJson)
            ?: error("Rust source host returned a null response")
        return try {
            response.toKString()
        } finally {
            legado_source_host_string_free(response)
        }
    }
}

/** Swift 直接调用的 Kotlin/Native framework 入口。 */
object IosSourceEngine {
    suspend fun execute(call: SourceEngineCall, appDataDirectory: String): String {
        IosSourceEngineRuntime.install()
        return WebBookSourceRuleExecutor.execute(call, IosRustSourceEngineHost(appDataDirectory))
    }
}
