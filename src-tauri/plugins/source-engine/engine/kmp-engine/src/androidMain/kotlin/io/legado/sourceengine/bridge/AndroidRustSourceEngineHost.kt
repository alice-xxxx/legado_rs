package io.legado.sourceengine.bridge

import io.legado.sourceengine.android.rustSourceImageRequest

import io.legado.app.data.entities.Book

import io.legado.sourceengine.android.rustSourceHttpRequest
import io.legado.sourceengine.android.rustSourceStorageRequest
import io.legado.app.help.file.registerAndroidAppFilesDir
import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Android 宿主适配器：应用提供私有数据目录，每次存储请求都显式携带，避免并发书源操作
 * 争用 Rust 进程级路径。网络和存储实际由同一 Rust crate 执行。
 */
class AndroidRustSourceEngineHost(
    appDataDirectory: String,
    private val browserContext: Context?,
) : SourceEngineHost {
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

    override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse = withContext(Dispatchers.IO) {
        SourceEngineHostWire.decodeHttpResponse(
            rustSourceHttpRequest(SourceEngineHostWire.encodeHttpRequest(request)),
            request.url,
        )
    }

    override suspend fun executeBrowser(request: HostBrowserRequest): HostBrowserResponse {
        val context = requireNotNull(browserContext) {
            "Android private source browser requires the native source-engine plugin context"
        }
        return io.legado.sourceengine.android.renderSourceBrowserSnapshot(context, request)
    }

    private suspend fun storageCall(requestJson: String): String? = withContext(Dispatchers.IO) {
        SourceEngineHostWire.decodeStorageResponse(
            rustSourceStorageRequest(requestJson),
        )
    }
}

/** Android entry points share the JVM DTOs and rule implementation. */
object AndroidSourceEngine {
    private suspend fun <T> call(appDataDirectory: String, executionId: String, browserContext: Context?, block: suspend (SourceEngineHost) -> T): T {
        browserContext?.let { registerAndroidAppFilesDir(it, appDataDirectory) }
        AndroidRustSourceHttpProvider.install()
        JvmSourceEngineRuntime.install(::rustSourceImageRequest)
        return block(AndroidRustSourceEngineHost(appDataDirectory, browserContext).forExecution(executionId))
    }

    suspend fun search(request: SearchRequest, appDataDirectory: String, executionId: String, browserContext: Context?): SearchPage =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.search(request, it) }

    suspend fun exploreKinds(request: SourceRequest, appDataDirectory: String, executionId: String, browserContext: Context?): ExploreKinds =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.exploreKinds(request, it) }

    suspend fun explore(request: ExploreRequest, appDataDirectory: String, executionId: String, browserContext: Context?): SearchPage =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.explore(request, it) }

    suspend fun detail(request: BookRequest, appDataDirectory: String, executionId: String, browserContext: Context?): Book =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.detail(request, it) }

    suspend fun chapters(request: BookRequest, appDataDirectory: String, executionId: String, browserContext: Context?): ChapterList =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.chapters(request, it) }

    suspend fun content(request: ContentRequest, appDataDirectory: String, executionId: String, browserContext: Context?): ChapterContent =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.content(request, it) }

    suspend fun resolveMedia(request: MediaRequest, appDataDirectory: String, executionId: String, browserContext: Context?): MediaLocation =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.resolveMedia(request, it) }

    suspend fun httpTtsAudio(request: TtsRequest, appDataDirectory: String, executionId: String, browserContext: Context?): AudioData =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.httpTtsAudio(request, it) }

    suspend fun login(request: LoginRequest, appDataDirectory: String, executionId: String, browserContext: Context?): LoginState =
        call(appDataDirectory, executionId, browserContext) { WebBookSourceRuleExecutor.login(request, it) }
}

/** Tauri's Java commands run these methods on their worker executor. */
object AndroidSourceEngineBlocking {
    @JvmStatic
    fun search(request: SearchRequest, appDataDirectory: String, executionId: String, browserContext: Context?): SearchPage =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.search(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun exploreKinds(request: SourceRequest, appDataDirectory: String, executionId: String, browserContext: Context?): ExploreKinds =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.exploreKinds(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun explore(request: ExploreRequest, appDataDirectory: String, executionId: String, browserContext: Context?): SearchPage =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.explore(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun detail(request: BookRequest, appDataDirectory: String, executionId: String, browserContext: Context?): Book =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.detail(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun chapters(request: BookRequest, appDataDirectory: String, executionId: String, browserContext: Context?): ChapterList =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.chapters(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun content(request: ContentRequest, appDataDirectory: String, executionId: String, browserContext: Context?): ChapterContent =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.content(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun resolveMedia(request: MediaRequest, appDataDirectory: String, executionId: String, browserContext: Context?): MediaLocation =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.resolveMedia(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun httpTtsAudio(request: TtsRequest, appDataDirectory: String, executionId: String, browserContext: Context?): AudioData =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.httpTtsAudio(request, appDataDirectory, executionId, browserContext) }

    @JvmStatic
    fun login(request: LoginRequest, appDataDirectory: String, executionId: String, browserContext: Context?): LoginState =
        kotlinx.coroutines.runBlocking { AndroidSourceEngine.login(request, appDataDirectory, executionId, browserContext) }
}
