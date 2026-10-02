package io.legado.sourceengine.bridge

import io.legado.app.constant.AppConst
import io.legado.app.data.AppDbAccessor
import io.legado.app.data.AppDbProviders
import io.legado.app.data.dao.BookChapterDao
import io.legado.app.data.dao.CacheDao
import io.legado.app.data.dao.CookieDao
import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.Cache
import io.legado.app.data.entities.Cookie
import io.legado.app.data.entities.ReplaceRule
import io.legado.app.help.ExploreKindsCacheProvider
import io.legado.app.help.ExploreKindsCacheProviders
import io.legado.app.help.FileCacheProvider
import io.legado.app.help.FileCacheProviders
import io.legado.app.help.JsCryptoProviderJvm
import io.legado.app.help.JsCryptoProviders
import io.legado.app.help.RuleBigDataProvider
import io.legado.app.help.RuleBigDataProviders
import io.legado.app.help.UserAgentProvider
import io.legado.app.help.UserAgentProviders
import io.legado.app.help.book.BookHelpAccessor
import io.legado.app.help.book.BookHelpProviders
import io.legado.app.help.book.ContentProcessorAccessor
import io.legado.app.help.book.ContentProcessorProviders
import io.legado.app.help.config.AppConfigAccessor
import io.legado.app.help.config.AppConfigProviders
import io.legado.app.help.config.PreferenceProvider
import io.legado.app.help.config.PreferenceProviders
import io.legado.app.help.http.CookieStoreProviders
import io.legado.app.help.http.SharedCookieStore
import io.legado.app.help.http.newCallStrResponse
import io.legado.app.help.http.registerSharedCookieJarBridge
import io.legado.app.help.source.SourceCacheProvider
import io.legado.app.help.source.SourceCacheProviders
import io.legado.app.help.source.SourceDebugLogger
import io.legado.app.help.source.SourceDebugLoggers
import io.legado.app.help.source.SourceNetworkProvider
import io.legado.app.help.source.SourceNetworkProviders
import io.legado.app.model.SharedJsScope
import io.legado.app.model.script.JsBindingInjector
import io.legado.app.model.script.JsEngineType
import io.legado.app.model.script.JsEngines
import io.legado.app.model.script.quickjs.QuickJsJsEngine
import io.legado.app.model.script.quickjs.QuickJsSharedJsScopeBase
import io.legado.app.utils.RegexReplacerImpl
import io.legado.app.utils.RegexReplacers
import io.legado.app.exception.NoStackTraceException
import io.legado.app.utils.KS_JSON
import io.legado.sourceengine.bridge.http.RustHostHttpProvider
import kotlinx.coroutines.runBlocking
import io.legado.app.help.CacheManager
import java.lang.reflect.InvocationHandler
import java.lang.reflect.Method
import java.lang.reflect.Proxy
import java.nio.charset.StandardCharsets
import java.util.Base64
import java.util.concurrent.ConcurrentHashMap

/**
 * 为桌面 Rust/JNI 宿主注册解析器依赖的 provider；不启动原应用的 DesktopCore。
 *
 * 来源标记：本文件是为独立 Rust/JNI 宿主新写的平台适配层，不是原 Android/iOS/桌面 provider 的复制件。
 * 它实现/注册的接口来自仓库原 data 模块（AppDbAccessor、PreferenceProvider、
 * SourceNetworkProvider、SourceCacheProvider、SharedCookieStore 等）；具体来源见本模块 README。
 * 原平台的 provider 注册入口位于 desktop-core，本适配器不加载那些实现，而在这里接 Rust RPC
 * 或请求内存态，避免启动原平台的数据库、配置与网络服务。
 *
 * 这个边界是有意收窄的：DesktopCore 会带入桌面端数据库、文件与其他平台服务；独立宿主的
 * 系统 I/O 由 Rust 负责。需要跨操作保留的缓存、Cookie 和变量通过 JNI 存储回调进入 Rust；当前请求内
 * 仅为解析器兼容而提供少量内存状态。章节 DAO 不连接书架数据库，因为这里的职责是返回解析
 * 结果，而不是修改用户书架或复用另一平台的本地状态。
 */
object RustSourceEngineProviders {

    fun install() {
        // 规则解析会读取应用配置接口；无界面宿主没有 UI 设置页，因此使用明确的执行默认值，
        // 而不是隐式启动桌面 Preference/Room 实现。
        PreferenceProviders.register(RustHostPreferenceProvider)
        AppConfigProviders.register(rustHostAppConfig())
        // 仅注册 provider 接口兼容层。真正会跨请求保存的数据由下面的 Rust storage provider 处理。
        AppDbProviders.register(rustHostAppDb())
        BookHelpProviders.register(RustHostBookHelp)
        ContentProcessorProviders.register(RustHostContentProcessor)
        FileCacheProviders.impl = RustHostFileCache
        SourceCacheProviders.impl = RustHostSourceCache
        ExploreKindsCacheProviders.impl = RustHostExploreKindsCache
        RuleBigDataProviders.impl = RustHostRuleBigData
        SourceDebugLoggers.impl = RustHostSourceDebugLogger

        // 复用 shared cookie 语义，但 Cookie 的持久化由 DAO 适配器转发给 Rust。
        CookieStoreProviders.register(SharedCookieStore)
        registerSharedCookieJarBridge()
        SourceNetworkProviders.impl = RustHostSourceNetwork
        UserAgentProviders.impl = UserAgentProvider { AppConst.DEFAULT_USER_AGENT }
        // 显式注册替换规则，避免独立 Rust/JNI 宿主依赖原应用的启动副作用。
        RegexReplacers.register(RegexReplacerImpl)
        JsCryptoProviders.register(JsCryptoProviderJvm)

        // QuickJS 属于解析器能力，不是桌面服务。Rust 创建 JVM 时设置 native-library 路径，
        // JVM 在进程内加载 QuickJS JNI 库，供规则中的 JavaScript 执行使用。
        JsEngines.registerProvider { type ->
            check(type == JsEngineType.QUICKJS)
            QuickJsJsEngine
        }
        SharedJsScope.registerProviders { type ->
            check(type == JsEngineType.QUICKJS)
            SourceEngineQuickJsSharedJsScope
        }
        JsBindingInjector.registerImageOps(UnsupportedImageOps)
    }
}

private val SourceEngineQuickJsSharedJsScope = object : QuickJsSharedJsScopeBase() {
    private val jsLibContent = ConcurrentHashMap<String, String>()

    override fun cachedJsLibContent(name: String): String? = jsLibContent[name]

    override fun storeJsLibContent(name: String, content: String) {
        jsLibContent[name] = content
    }

    override fun downloadJsLibContent(url: String): String? = runBlocking {
        RustHostHttpProvider.okHttpClient.newCallStrResponse { url(url) }.body
    }

    override fun jsLibDownloadFailedException(url: String): Exception =
        NoStackTraceException("Failed to download source jsLib: $url")
}

private object RustHostPreferenceProvider : PreferenceProvider {
    private val values = ConcurrentHashMap<String, Any>()
    private val listeners = ConcurrentHashMap.newKeySet<(String) -> Unit>()

    override fun getString(key: String, default: String): String = values[key] as? String ?: default
    override fun getStringOrNull(key: String): String? = values[key] as? String
    override fun getInt(key: String, default: Int): Int = values[key] as? Int ?: default
    override fun getBoolean(key: String, default: Boolean): Boolean = values[key] as? Boolean ?: default
    override fun getLong(key: String, default: Long): Long = values[key] as? Long ?: default
    override fun getFloat(key: String, default: Float): Float = values[key] as? Float ?: default

    override fun putString(key: String, value: String?) = put(key, value)
    override fun putInt(key: String, value: Int) = put(key, value)
    override fun putBoolean(key: String, value: Boolean) = put(key, value)
    override fun putLong(key: String, value: Long) = put(key, value)
    override fun putFloat(key: String, value: Float) = put(key, value)
    override fun remove(key: String) {
        values.remove(key)
        notifyChanged(key)
    }
    override fun contains(key: String): Boolean = values.containsKey(key)
    override fun getAll(): Map<String, *> = values.toMap()
    override fun addPreferenceChangeListener(listener: (key: String) -> Unit): () -> Unit {
        listeners += listener
        return { listeners -= listener }
    }

    private fun put(key: String, value: Any?) {
        if (value == null) values.remove(key) else values[key] = value
        notifyChanged(key)
    }

    private fun notifyChanged(key: String) = listeners.forEach { it(key) }
}

private fun rustHostAppConfig(): AppConfigAccessor = Proxy.newProxyInstance(
    AppConfigAccessor::class.java.classLoader,
    arrayOf(AppConfigAccessor::class.java),
    InvocationHandler { proxy, method, args ->
        when (method.name) {
            "getThreadCount" -> 16
            // 没有书架章节列表可用于统计字数；返回 false 可让目录解析保持纯网络输入，不触碰用户书架。
            "getTocCountWords" -> false
            "getPrecisionSearch" -> false
            "getChineseConverterType" -> 0
            "getReplaceEnableDefault" -> true
            "getTocUiUseReplace" -> false
            "toString" -> "RustHostAppConfigAccessor"
            "hashCode" -> System.identityHashCode(proxy)
            "equals" -> proxy === args?.firstOrNull()
            else -> when (method.returnType) {
                java.lang.Boolean.TYPE -> false
                java.lang.Integer.TYPE -> 0
                java.lang.Long.TYPE -> 0L
                java.lang.Float.TYPE -> 0f
                java.lang.Double.TYPE -> 0.0
                java.lang.Void.TYPE -> null
                String::class.java -> ""
                List::class.java, Set::class.java -> emptyList<Any>()
                Map::class.java -> emptyMap<Any, Any>()
                else -> error("Unsupported source-engine config access: ${method.name}")
            }
        }
    },
) as AppConfigAccessor

private fun rustHostAppDb(): AppDbAccessor = Proxy.newProxyInstance(
    AppDbAccessor::class.java.classLoader,
    arrayOf(AppDbAccessor::class.java),
    InvocationHandler { proxy, method, args ->
        when {
            method.name == "toString" -> "RustBackedSourceEngineAppDbAccessor"
            method.name == "hashCode" -> System.identityHashCode(proxy)
            method.name == "equals" -> proxy === args?.firstOrNull()
            // DAO 只在解析器确实访问时创建代理；未覆盖的方法在 daoProxy 中明确报错，
            // 防止新引入的平台依赖悄悄被空值吞掉。
            method.name.startsWith("get") && method.returnType.isInterface ->
                daoProxy(method.returnType)
            else -> error("Unsupported source-engine database access: ${method.name}")
        }
    },
) as AppDbAccessor

private fun daoProxy(daoType: Class<*>): Any = Proxy.newProxyInstance(
    daoType.classLoader,
    arrayOf(daoType),
    InvocationHandler { proxy, method, rawArgs ->
        val args = rawArgs.withoutContinuation(method)
        when (daoType) {
            BookChapterDao::class.java -> when (method.name) {
                "getChapter" -> null
                "getChapterList" -> emptyList<BookChapter>()
                "upTitle" -> Unit
                "toString" -> "SourceEngineChapterDao"
                else -> error("Unsupported chapter DAO access in source parser: ${method.name}")
            }

            CacheDao::class.java -> cacheDaoCall(method, args)
            CookieDao::class.java -> cookieDaoCall(method, args)
            else -> when (method.name) {
                "toString" -> "UnsupportedSourceEngineDao(${daoType.simpleName})"
                else -> error("Unsupported DAO access in source parser: ${daoType.simpleName}.${method.name}")
            }
        }
    },
)

private fun cacheDaoCall(method: Method, args: List<Any?>): Any? = when (method.name) {
    "get" -> {
        val key = args[0] as String
        RustHostHttpProvider.storage("source-cache", "get", key)?.let { encoded ->
            val separator = encoded.indexOf('\n')
            check(separator >= 0) { "Invalid cached source value for key '$key'" }
            Cache(
                key = key,
                deadline = encoded.substring(0, separator).toLong(),
                value = encoded.substring(separator + 1),
            )
        }
    }

    "insert" -> {
        val caches = args[0] as Array<*>
        caches.filterIsInstance<Cache>().forEach { cache ->
            val deadline = cache.deadline
            val ttl = if (deadline <= 0L) 0 else ((deadline - System.currentTimeMillis()) / 1000L)
                .coerceAtLeast(1L).coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
            RustHostHttpProvider.storage(
                namespace = "source-cache",
                operation = "put",
                key = cache.key,
                value = "${cache.deadline}\n${cache.value.orEmpty()}",
                ttlSeconds = ttl,
            )
        }
        Unit
    }

    "delete" -> {
        RustHostHttpProvider.storage("source-cache", "delete", args[0] as String)
        Unit
    }
    "deleteSourceVariables" -> {
        val source = args[0] as String
        RustHostHttpProvider.storage(
            "source-cache",
            "deletePrefixes",
            keyPrefixes = listOf("v_${source}_", "userInfo_$source", "loginHeader_$source", "sourceVariable_$source"),
        )
        Unit
    }
    "clearDeadline" -> Unit
    "toString" -> "RustBackedSourceCacheDao"
    else -> error("Unsupported cache DAO access: ${method.name}")
}

private fun cookieDaoCall(method: Method, args: List<Any?>): Any? = when (method.name) {
    "get" -> {
        val key = args[0] as String
        RustHostHttpProvider.storage("cookies", "get", key)?.let { Cookie(url = key, cookie = it) }
    }
    "insert" -> {
        (args[0] as Array<*>).filterIsInstance<Cookie>().forEach { cookie ->
            RustHostHttpProvider.storage("cookies", "put", cookie.url, cookie.cookie)
        }
        Unit
    }
    "delete" -> {
        RustHostHttpProvider.storage("cookies", "delete", args[0] as String)
        Unit
    }
    "deleteOkHttp" -> {
        RustHostHttpProvider.storage("cookies", "deleteContains", contains = "|")
        Unit
    }
    "toString" -> "RustBackedSourceCookieDao"
    else -> error("Unsupported cookie DAO access: ${method.name}")
}

private fun Array<out Any?>?.withoutContinuation(method: Method): List<Any?> {
    val values = this?.toList().orEmpty()
    return if (method.parameterTypes.lastOrNull()?.name == "kotlin.coroutines.Continuation") {
        values.dropLast(1)
    } else values
}

private object RustHostBookHelp : BookHelpAccessor {
    override fun saveContent(bookSource: io.legado.app.data.entities.BookSource, book: Book, bookChapter: BookChapter, content: String) {
        // content 操作在入口显式设置 needSave=false；若未来调用路径误开保存，宁可立即失败也不静默丢失。
        error("Source-engine content requests must not persist chapters")
    }

    override fun getCoverPath(bookUrl: String): String =
        error("The parser does not manage cover files")

    override suspend fun clearCacheExtra() = Unit
}

private object RustHostContentProcessor : ContentProcessorAccessor {
    override fun getTitleReplaceRules(book: Book): List<ReplaceRule> = emptyList()
    override fun getContent(book: Book, chapter: BookChapter, content: String, useReplace: Boolean): CharSequence = content
    override fun upReplaceRules() = Unit
}

private object RustHostFileCache : FileCacheProvider {
    override fun put(key: String, value: String, saveTime: Int, persistent: Boolean) {
        RustHostHttpProvider.storage(fileCacheNamespace(persistent), "put", key, value, saveTime)
    }

    override fun getAsString(key: String, persistent: Boolean): String? =
        RustHostHttpProvider.storage(fileCacheNamespace(persistent), "get", key)

    override fun put(key: String, value: ByteArray, saveTime: Int, persistent: Boolean) {
        put(key, "base64:${Base64.getEncoder().encodeToString(value)}", saveTime, persistent)
    }

    override fun getAsBinary(key: String, persistent: Boolean): ByteArray? {
        val encoded = getAsString(key, persistent) ?: return null
        // 字符串缓存和二进制缓存共用 Rust 的字符串存储；显式前缀用于区分格式，
        // 避免把普通文本误当 Base64 解码后返回损坏数据。
        if (!encoded.startsWith("base64:")) return null
        return Base64.getDecoder().decode(encoded.removePrefix("base64:"))
    }

    override fun remove(key: String, persistent: Boolean) {
        RustHostHttpProvider.storage(fileCacheNamespace(persistent), "delete", key)
    }

    private fun fileCacheNamespace(persistent: Boolean) = if (persistent) "persistent-file-cache" else "file-cache"
}

private object RustHostSourceCache : SourceCacheProvider {
    override fun get(key: String): String? = CacheManager.get(key)
    override fun put(key: String, value: String) = CacheManager.put(key, value)
    override fun put(key: String, value: String, saveTime: Int) = CacheManager.put(key, value, saveTime)
    override fun delete(key: String) = CacheManager.delete(key)
    override fun getFromMemory(key: String): Any? = CacheManager.getFromMemory(key)
    override fun putMemory(key: String, value: Any) = CacheManager.putMemory(key, value)
    override fun deleteMemory(key: String) = CacheManager.deleteMemory(key)
    override fun clearMemoryByPrefixes(prefixes: List<String>) = Unit
    override fun asBinding(): Any = CacheManager
}

private object RustHostExploreKindsCache : ExploreKindsCacheProvider {
    override fun getAsString(key: String): String? = RustHostHttpProvider.storage("explore-kinds", "get", key)
    override fun put(key: String, value: String) { RustHostHttpProvider.storage("explore-kinds", "put", key, value) }
    override fun remove(key: String) { RustHostHttpProvider.storage("explore-kinds", "delete", key) }
}

private object RustHostRuleBigData : RuleBigDataProvider {
    override fun putBookVariable(bookUrl: String, key: String, value: String?) =
        put(bigDataKey(bookUrl, key), value)

    override fun getBookVariable(bookUrl: String, key: String?): String? =
        get(bigDataKey(bookUrl, key.orEmpty()))

    override fun hasBookVariable(bookUrl: String, key: String): Boolean =
        get(bigDataKey(bookUrl, key)) != null

    override fun putChapterVariable(bookUrl: String, chapterUrl: String, key: String, value: String?) =
        put(bigDataKey(bookUrl, chapterUrl, key), value)

    override fun getChapterVariable(bookUrl: String, chapterUrl: String, key: String): String? =
        get(bigDataKey(bookUrl, chapterUrl, key))

    override fun listBookDataDirs(): List<String> = emptyList()
    override fun clearInvalidBookData(bookUrls: Set<String>) = Unit

    private fun put(key: String, value: String?) {
        RustHostHttpProvider.storage("book-variables", if (value == null) "delete" else "put", key, value)
    }

    private fun get(key: String): String? = RustHostHttpProvider.storage("book-variables", "get", key)

    private fun bigDataKey(vararg parts: String): String = parts.joinToString(":") {
        Base64.getUrlEncoder().withoutPadding().encodeToString(it.toByteArray(StandardCharsets.UTF_8))
    }
}

private object RustHostSourceNetwork : SourceNetworkProvider {
    override fun getCookie(tag: String): String = SharedCookieStore.getCookie(tag)
    override fun replaceCookie(tag: String, cookie: String) = SharedCookieStore.replaceCookie(tag, cookie)
    override fun removeCookie(tag: String) = SharedCookieStore.removeCookie(tag)
    override fun asBinding(): Any = SharedCookieStore
}

private object RustHostSourceDebugLogger : SourceDebugLogger {
    override fun log(key: String, msg: String, print: Boolean, state: Int, showTime: Boolean) {
        if (!print) return
        val line = msg.replace('\n', ' ').take(4096)
        System.err.println("[source:$state] $key $line")
    }

    override fun log(msg: String) = log("", msg)
}

private object UnsupportedImageOps : io.legado.app.help.image.ImageOps {
    override fun decode(bytes: ByteArray): io.legado.app.help.image.ImageRef = unsupported()
    override fun decode(base64: String): io.legado.app.help.image.ImageRef = unsupported()
    override fun encode(img: io.legado.app.help.image.ImageRef, format: String, quality: Int): ByteArray = unsupported()
    override fun split(img: io.legado.app.help.image.ImageRef, rows: Int, cols: Int): List<io.legado.app.help.image.ImageRef> = unsupported()
    override fun stitch(imgs: List<io.legado.app.help.image.ImageRef>, direction: String): io.legado.app.help.image.ImageRef = unsupported()
    override fun crop(img: io.legado.app.help.image.ImageRef, x: Int, y: Int, w: Int, h: Int): io.legado.app.help.image.ImageRef = unsupported()
    override fun rotate(img: io.legado.app.help.image.ImageRef, deg: Int): io.legado.app.help.image.ImageRef = unsupported()
    override fun flip(img: io.legado.app.help.image.ImageRef, direction: String): io.legado.app.help.image.ImageRef = unsupported()
    override fun size(img: io.legado.app.help.image.ImageRef): Map<String, Int> = unsupported()

    private fun <T> unsupported(): T = error("The Rust source host does not provide image decoding")
}
