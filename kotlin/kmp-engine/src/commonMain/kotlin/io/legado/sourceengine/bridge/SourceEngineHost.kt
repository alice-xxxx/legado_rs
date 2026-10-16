package io.legado.sourceengine.bridge

import kotlin.coroutines.AbstractCoroutineContextElement
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Runnable
import io.legado.app.help.coroutine.IoDispatcher
import kotlinx.coroutines.withContext
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.job
import kotlinx.coroutines.CancellationException

/**
 * 书源执行器传给 Rust 宿主的跨平台 HTTP 请求。
 *
 * Kotlin 负责生成 URL、请求头、执行规则和解析响应。Rust 负责建立网络连接、处理重定向、代理及
 * Cookie Jar，避免书源绕过宿主而直接使用平台 HTTP 客户端。
 */
data class HostHttpRequest(
    val url: String,
    val method: String = "GET",
    val headers: List<HostHeader> = emptyList(),
    val body: ByteArray? = null,
    val multipart: List<HostHttpMultipartPart> = emptyList(),
    val connectTimeoutMs: Long? = null,
    val readTimeoutMs: Long? = null,
    val callTimeoutMs: Long? = null,
    val maxRedirects: Int? = null,
    val useCookieJar: Boolean = false,
    val cookieScope: String? = null,
    val proxy: String? = null,
    val acceptInvalidCerts: Boolean = false,
    val executionId: String? = null,
)

data class HostHttpMultipartPart(
    val name: String,
    val text: String? = null,
    val fileName: String? = null,
    val contentType: String? = null,
    val bytes: ByteArray? = null,
)

data class HostHttpResponse(
    val requestedUrl: String,
    val finalUrl: String,
    val status: Int,
    val reason: String,
    val redirects: List<HostHttpRedirect> = emptyList(),
    val headers: List<HostHeader> = emptyList(),
    val body: ByteArray = byteArrayOf(),
)

enum class HostBrowserMode {
    /** Inert HTML snapshot with network and page code disabled. */
    OFFLINE_SNAPSHOT,

    /** Real desktop page navigation inside a private, incognito source browser. */
    DYNAMIC_PAGE,

    /** 用户可见的书源浏览器；关闭时回传正文、地址和 Cookie。 */
    INTERACTIVE_PAGE,
}

/** Private browser operation. Dynamic mode is desktop-only and must never be emulated by snapshots. */
data class HostBrowserRequest(
    val capabilityId: String,
    val html: String,
    val baseUrl: String,
    val javaScript: String,
    val delayTimeMs: Long = 0,
    val mode: HostBrowserMode = HostBrowserMode.OFFLINE_SNAPSHOT,
    val method: String = "GET",
    val headers: List<HostHeader> = emptyList(),
    val useCookieJar: Boolean = false,
    val cookieScope: String? = null,
    val title: String = "",
    val waitForResult: Boolean = true,
    val executionId: String? = null,
)

data class HostBrowserResponse(
    val capabilityId: String,
    val finalUrl: String,
    val body: String,
    val cookieHeader: String = "",
)

/** A real redirect hop followed by Rust's HTTP client for this request. */
data class HostHttpRedirect(
    val fromUrl: String,
    val toUrl: String,
    val status: Int,
)

data class HostHeader(val name: String, val value: String)

/**
 * 书源的持久状态和内存状态均通过 Rust 访问，不直接使用应用数据库或文件 API。
 * 命名空间白名单和过期策略由 Rust 实现强制执行。
 */
interface HostStorage {
    suspend fun get(namespace: String, key: String): String?
    suspend fun put(namespace: String, key: String, value: String, ttlSeconds: Long? = null)
    suspend fun delete(namespace: String, key: String)
    suspend fun deletePrefixes(namespace: String, prefixes: List<String>)
    suspend fun deleteContains(namespace: String, value: String)
    suspend fun clear(namespace: String)
    suspend fun hasCookieJarCookies(scope: String, sourceUrl: String): Boolean
}

/**
 * 解析器使用的宿主回调，由各平台的 Rust 适配层提供。
 *
 * KMP 接口使用 suspend 函数；平台适配器通过 JNI 或 C ABI 调用 Rust Host，
 * 规则执行和网络调度可在协程中取消。
 */
interface SourceEngineHost {
    val executionId: String? get() = null
    suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse
    suspend fun executeBrowser(request: HostBrowserRequest): HostBrowserResponse
    /** False by default so mobile hosts never silently substitute an offline snapshot. */
    val supportsDynamicBrowser: Boolean get() = false
    val storage: HostStorage
}

/** 桌面、Android 和 iOS 适配层共用的书源操作参数。 */
data class SourceEngineCall(
    val operation: String,
    val sourceJson: String,
    val keyword: String? = null,
    val credentials: Map<String, String>? = null,
    val actionId: Int? = null,
    val mediaUrl: String? = null,
    val mediaHeaders: Map<String, String>? = null,
    val page: Int? = null,
    val bookJson: String? = null,
    val chapterJson: String? = null,
    val nextChapterUrl: String? = null,
    val executionId: String? = null,
)

/**
 * 由 KMP WebBook/AnalyzeRuleCore source set 实现的解析入口。
 * 显式传入宿主，避免解析器隐式访问平台单例。
 */
interface SourceRuleExecutor {
    suspend fun execute(call: SourceEngineCall, host: SourceEngineHost): String
}

/**
 * 单次书源调用的宿主上下文。
 *
 * 普通 HTTP 请求把宿主放进协程上下文，再由 AnalyzeUrlCore 写入请求 tag，确保 OkHttp worker
 * 线程仍能找到发起调用的 Rust Host。同步 JS 绑定通过调度器维护的线程局部宿主取上下文，
 * 每次协程恢复时安装、挂起时还原，避免网络等待期间占用其他书源的执行位置。
 */
internal class SourceEngineHostContext(
    val host: SourceEngineHost,
) : AbstractCoroutineContextElement(Key) {
    companion object Key : CoroutineContext.Key<SourceEngineHostContext>
}

/**
 * 规则执行共享安全的调度通道，宿主上下文随每次调用独立传递；HTTP 等待可相互重叠。
 */
object SourceEngineHostRegistry {
    // Native QuickJS has synchronous process-wide binding state. Only rule work uses this
    // lane; HTTP suspends on the platform I/O dispatcher, releasing it for another source.
    private val ruleDispatcher = IoDispatcher.limitedParallelism(1)
    fun current(): SourceEngineHost? = sourceHostOnThread()
    @kotlin.jvm.JvmStatic
    fun executionCancelled(): Boolean = current()?.executionId?.let(::sourceExecutionCancelled) ?: false

    internal fun <T> onThread(host: SourceEngineHost, block: () -> T): T {
        val previous = sourceHostOnThread()
        setSourceHostOnThread(host)
        return try { block() } finally { setSourceHostOnThread(previous) }
    }

    internal suspend fun <T> withHost(host: SourceEngineHost, block: suspend () -> T): T =
        withContext(object : CoroutineDispatcher() {
            override fun dispatch(context: CoroutineContext, block: Runnable) {
                ruleDispatcher.dispatch(context, Runnable { onThread(host) { block.run() } })
            }
        }) { block() }
}

internal expect fun sourceHostOnThread(): SourceEngineHost?
internal expect fun setSourceHostOnThread(host: SourceEngineHost?)
internal expect fun sourceExecutionCancelled(id: String): Boolean

internal fun SourceEngineHost.forExecution(id: String?): SourceEngineHost {
    val delegate = this
    return object : SourceEngineHost by delegate {
        override val executionId = id
        override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse =
            delegate.executeHttp(request.copy(executionId = id))
        override suspend fun executeBrowser(request: HostBrowserRequest): HostBrowserResponse =
            coroutineScope {
                // Mobile browser renderers suspend outside Rust's HTTP runtime. Forward
                // this lease's cancellation to their Job so WebKit/WebView cleanup runs.
                val execution = coroutineContext.job
                val cancellation = id?.let { executionId ->
                    launch(Dispatchers.Default) {
                        while (isActive) {
                            if (sourceExecutionCancelled(executionId)) {
                                execution.cancel(CancellationException("Source execution cancelled"))
                                break
                            }
                            delay(50)
                        }
                    }
                }
                try { delegate.executeBrowser(request.copy(executionId = id)) }
                finally { cancellation?.cancel() }
            }
    }
}
