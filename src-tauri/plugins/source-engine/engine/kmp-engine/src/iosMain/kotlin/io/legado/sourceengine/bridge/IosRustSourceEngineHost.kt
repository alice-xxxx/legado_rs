@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.sourceengine.bridge

import io.legado.app.data.entities.Book

import io.legado.sourceengine.rust.legado_source_host_http
import io.legado.sourceengine.rust.legado_source_host_image
import io.legado.sourceengine.rust.legado_source_host_storage
import io.legado.sourceengine.rust.legado_source_host_string_free
import io.legado.app.help.file.registerIosAppFilesDir
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

        override suspend fun hasCookieJarCookies(scope: String, sourceUrl: String): Boolean =
            storageCall(
                SourceEngineHostWire.encodeStorageRequest(
                    "cookies", "jarHasCookies", key = scope, value = sourceUrl,
                ),
            ) == "true"
    }

    override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse = withContext(Dispatchers.Default) {
        SourceEngineHostWire.decodeHttpResponse(
            rustResponse(requestJson = SourceEngineHostWire.encodeHttpRequest(request)) { input ->
                legado_source_host_http(input)
            },
            request.url,
        )
    }

    override suspend fun executeBrowser(request: HostBrowserRequest): HostBrowserResponse =
        renderIosSourceBrowserSnapshot(request)

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

/** Synchronous image pixel RPC used from ImageOps while a JS rule is running. */
internal fun rustImageRequest(requestJson: String): String {
    val response = legado_source_host_image(requestJson)
        ?: error("Rust image host returned a null response")
    return try {
        response.toKString()
    } finally {
        legado_source_host_string_free(response)
    }
}

/** Swift calls typed framework methods; exceptions become normal native errors. */
object IosSourceEngine {
    private suspend fun <T> call(appDataDirectory: String, executionId: String, block: suspend (SourceEngineHost) -> T): T {
        registerIosAppFilesDir(appDataDirectory)
        IosSourceEngineRuntime.install()
        return block(IosRustSourceEngineHost(appDataDirectory).forExecution(executionId))
    }

    @Throws(Exception::class)
    suspend fun search(request: SearchRequest, appDataDirectory: String, executionId: String): SearchPage =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.search(request, it) }

    @Throws(Exception::class)
    suspend fun exploreKinds(request: SourceRequest, appDataDirectory: String, executionId: String): ExploreKinds =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.exploreKinds(request, it) }

    @Throws(Exception::class)
    suspend fun explore(request: ExploreRequest, appDataDirectory: String, executionId: String): SearchPage =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.explore(request, it) }

    @Throws(Exception::class)
    suspend fun detail(request: BookRequest, appDataDirectory: String, executionId: String): Book =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.detail(request, it) }

    @Throws(Exception::class)
    suspend fun chapters(request: BookRequest, appDataDirectory: String, executionId: String): ChapterList =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.chapters(request, it) }

    @Throws(Exception::class)
    suspend fun content(request: ContentRequest, appDataDirectory: String, executionId: String): ChapterContent =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.content(request, it) }

    @Throws(Exception::class)
    suspend fun resolveMedia(request: MediaRequest, appDataDirectory: String, executionId: String): MediaLocation =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.resolveMedia(request, it) }

    @Throws(Exception::class)
    suspend fun httpTtsAudio(request: TtsRequest, appDataDirectory: String, executionId: String): AudioData =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.httpTtsAudio(request, it) }

    @Throws(Exception::class)
    suspend fun login(request: LoginRequest, appDataDirectory: String, executionId: String): LoginState =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.login(request, it) }
}
