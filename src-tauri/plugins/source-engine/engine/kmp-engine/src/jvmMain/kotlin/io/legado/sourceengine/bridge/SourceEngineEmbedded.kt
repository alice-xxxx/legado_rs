package io.legado.sourceengine.bridge

import io.legado.sourceengine.bridge.http.nativeImageRequest

import io.legado.app.data.entities.Book
import io.legado.app.help.coroutine.registerJvmDebugState
import io.legado.app.help.file.AppFilesDir
import io.legado.app.help.file.AppFilesDirs
import io.legado.sourceengine.bridge.http.RustHostHttpProvider
import io.legado.sourceengine.bridge.http.nativeBrowserSnapshot
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.file.Files
import java.nio.file.Path

/** Typed JVM entry points; JNI constructs the request DTO before invoking them. */
object SourceEngineEmbedded {
    @JvmStatic fun describeError(error: Throwable): String = error.stackTraceToString()
    @Volatile private var initialized = false

    private fun <T> call(appDataDirectory: String, executionId: String, block: suspend (SourceEngineHost) -> T): T {
        initializeOnce()
        val root = Path.of(appDataDirectory).resolve("source-engine")
        AppFilesDirs.register(object : AppFilesDir {
            override val filesDir = Files.createDirectories(root.resolve("files")).toString()
            override val cacheDir = Files.createDirectories(root.resolve("cache")).toString()
            override val externalFilesDir: String? = null
            override val externalCacheDir: String? = null
        })
        return runBlocking { block(EmbeddedRustSourceEngineHost.forExecution(executionId)) }
    }

    @JvmStatic @Throws(Exception::class)
    fun search(request: SearchRequest, appDataDirectory: String, executionId: String): SearchPage =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.search(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun exploreKinds(request: SourceRequest, appDataDirectory: String, executionId: String): ExploreKinds =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.exploreKinds(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun explore(request: ExploreRequest, appDataDirectory: String, executionId: String): SearchPage =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.explore(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun detail(request: BookRequest, appDataDirectory: String, executionId: String): Book =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.detail(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun chapters(request: BookRequest, appDataDirectory: String, executionId: String): ChapterList =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.chapters(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun content(request: ContentRequest, appDataDirectory: String, executionId: String): ChapterContent =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.content(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun resolveMedia(request: MediaRequest, appDataDirectory: String, executionId: String): MediaLocation =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.resolveMedia(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun httpTtsAudio(request: TtsRequest, appDataDirectory: String, executionId: String): AudioData =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.httpTtsAudio(request, it) }

    @JvmStatic @Throws(Exception::class)
    fun login(request: LoginRequest, appDataDirectory: String, executionId: String): LoginState =
        call(appDataDirectory, executionId) { WebBookSourceRuleExecutor.login(request, it) }

    @Synchronized
    private fun initializeOnce() {
        if (initialized) return
        System.setProperty("java.awt.headless", "true")
        System.setProperty("file.encoding", "UTF-8")
        registerJvmDebugState(false)
        RustHostHttpProvider.installNative()
        JvmSourceEngineRuntime.install(::nativeImageRequest)
        initialized = true
    }
}

private object EmbeddedRustSourceEngineHost : SourceEngineHost {
    override val supportsDynamicBrowser: Boolean = true

    override val storage: HostStorage = object : HostStorage {
        override suspend fun get(namespace: String, key: String): String? =
            RustHostHttpProvider.storage(namespace, "get", key)

        override suspend fun put(namespace: String, key: String, value: String, ttlSeconds: Long?) {
            RustHostHttpProvider.storage(
                namespace = namespace,
                operation = "put",
                key = key,
                value = value,
                ttlSeconds = ttlSeconds?.coerceAtMost(Int.MAX_VALUE.toLong())?.toInt(),
            )
        }

        override suspend fun delete(namespace: String, key: String) {
            RustHostHttpProvider.storage(namespace, "delete", key)
        }

        override suspend fun deletePrefixes(namespace: String, prefixes: List<String>) {
            RustHostHttpProvider.storage(namespace, "deletePrefixes", keyPrefixes = prefixes)
        }

        override suspend fun deleteContains(namespace: String, value: String) {
            RustHostHttpProvider.storage(namespace, "deleteContains", contains = value)
        }

        override suspend fun clear(namespace: String) {
            RustHostHttpProvider.storage(namespace, "clear")
        }

        override suspend fun hasCookieJarCookies(scope: String, sourceUrl: String): Boolean =
            RustHostHttpProvider.storage("cookies", "jarHasCookies", scope, sourceUrl) == "true"
    }

    override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse =
        RustHostHttpProvider.executeHostHttp(request)

    override suspend fun executeBrowser(request: HostBrowserRequest): HostBrowserResponse = withContext(Dispatchers.IO) {
        SourceEngineHostWire.decodeBrowserResponse(
            nativeBrowserSnapshot(SourceEngineHostWire.encodeBrowserRequest(request)),
            request,
        )
    }
}
