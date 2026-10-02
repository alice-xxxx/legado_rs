package io.legado.sourceengine.bridge

import kotlin.coroutines.AbstractCoroutineContextElement
import kotlin.coroutines.CoroutineContext
import kotlin.concurrent.Volatile
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

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
    val proxy: String? = null,
    val acceptInvalidCerts: Boolean = false,
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
    val headers: List<HostHeader> = emptyList(),
    val body: ByteArray = byteArrayOf(),
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
}

/**
 * 解析器使用的宿主回调，由各平台的 Rust 适配层提供。
 *
 * KMP 接口使用 suspend 函数；平台适配器通过 JNI 或 C ABI 调用 Rust Host，
 * 规则执行和网络调度可在协程中取消。
 */
interface SourceEngineHost {
    suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse
    val storage: HostStorage
}

/** 桌面、Android 和 iOS 适配层共用的书源操作参数。 */
data class SourceEngineCall(
    val operation: String,
    val sourceJson: String,
    val keyword: String? = null,
    val page: Int? = null,
    val bookJson: String? = null,
    val chapterJson: String? = null,
    val nextChapterUrl: String? = null,
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
 * 线程仍能找到发起调用的 Rust Host。同步 JS 绑定拿不到协程上下文，因此另有串行作用域：
 * 仅在执行锁保护期间暂存当前 Host，并在 finally 中清除，避免并发书源操作串用宿主。
 */
internal class SourceEngineHostContext(
    val host: SourceEngineHost,
) : AbstractCoroutineContextElement(Key) {
    companion object Key : CoroutineContext.Key<SourceEngineHostContext>
}

/**
 * 同步 JS 绑定无法读取 suspend 调用的 CoroutineContext；在串行执行区间内，
 * 用这个只读当前宿主让 JS 库下载等辅助请求也复用同一个 Rust HTTP/存储边界。
 */
object SourceEngineHostRegistry {
    private val executionLock = Mutex()

    @Volatile
    private var activeHost: SourceEngineHost? = null

    fun current(): SourceEngineHost? = activeHost

    internal suspend fun <T> withHost(host: SourceEngineHost, block: suspend () -> T): T =
        executionLock.withLock {
            check(activeHost == null) { "Source-engine host scope cannot be nested" }
            activeHost = host
            try {
                block()
            } finally {
                activeHost = null
            }
        }
}
