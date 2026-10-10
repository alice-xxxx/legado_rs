package io.legado.sourceengine.android

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
import io.legado.sourceengine.bridge.SourceEngineHostRpc
import io.legado.app.help.CacheManager
import okio.ByteString.Companion.decodeBase64
import okio.ByteString.Companion.toByteString
import java.lang.reflect.InvocationHandler
import java.lang.reflect.Method
import java.lang.reflect.Proxy
import java.util.concurrent.ConcurrentHashMap

/**
 * 注册 Android 书源执行器需要的 provider 与默认值。
 *
 * DAO 和配置通过动态代理提供解析器需要的接口；QuickJS 与加解密使用 Android source set
 * actual。持久状态通过 SourceEngineHost 交给 Rust 保存。
 *
 * Rust 负责网络和宿主存储；解析器需要的临时状态保留在当前请求内。章节 DAO 不连接书架数据库，
 * 这个入口只返回解析结果。
 */
object AndroidSourceEngineRuntime {
    @Volatile
    private var initialized = false

    @Synchronized
    fun install() {
        if (initialized) return
        // 规则解析会读取应用配置接口；执行器使用明确默认值，不依赖界面配置状态。
        PreferenceProviders.register(HeadlessPreferenceProvider)
        AppConfigProviders.register(headlessAppConfig())
        // 注册解析器需要的 DAO 接口；跨请求保存的数据由 Rust storage provider 处理。
        AppDbProviders.register(headlessAppDb())
        BookHelpProviders.register(HeadlessBookHelp)
        ContentProcessorProviders.register(HeadlessContentProcessor)
        FileCacheProviders.impl = HeadlessFileCache
        SourceCacheProviders.impl = HeadlessSourceCache
        ExploreKindsCacheProviders.impl = HeadlessExploreKindsCache
        RuleBigDataProviders.impl = HeadlessRuleBigData
        SourceDebugLoggers.impl = HeadlessSourceDebugLogger

        // 复用 shared cookie 语义，但 Cookie 的持久化由 DAO 适配器转发给 Rust。
        CookieStoreProviders.register(SharedCookieStore)
        registerSharedCookieJarBridge()
        SourceNetworkProviders.impl = HeadlessSourceNetwork
        UserAgentProviders.impl = UserAgentProvider { AppConst.DEFAULT_USER_AGENT }
        JsCryptoProviders.register(JsCryptoProviderJvm)
        // 书源替换规则使用 Android target 的实现。
        RegexReplacers.register(RegexReplacerImpl)

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
        JsBindingInjector.registerImageOps(
            io.legado.sourceengine.bridge.RustImageOpsProvider(::rustSourceImageRequest),
        )
        initialized = true
    }
}

private val SourceEngineQuickJsSharedJsScope = object : QuickJsSharedJsScopeBase() {
    private val jsLibContent = ConcurrentHashMap<String, String>()

    override fun cachedJsLibContent(name: String): String? = jsLibContent[name]

    override fun storeJsLibContent(name: String, content: String) {
        jsLibContent[name] = content
    }

    override fun downloadJsLibContent(url: String): String = SourceEngineHostRpc.downloadText(url)

    override fun jsLibDownloadFailedException(url: String): Exception =
        NoStackTraceException("Failed to download source jsLib: $url")
}

private object HeadlessPreferenceProvider : PreferenceProvider {
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

private fun headlessAppConfig(): AppConfigAccessor = Proxy.newProxyInstance(
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
            "toString" -> "HeadlessAppConfigAccessor"
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

private fun headlessAppDb(): AppDbAccessor = Proxy.newProxyInstance(
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
        SourceEngineHostRpc.storage("source-cache", "get", key)?.let { encoded ->
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
            SourceEngineHostRpc.storage(
                namespace = "source-cache",
                operation = "put",
                key = cache.key,
                value = "${cache.deadline}\n${cache.value.orEmpty()}",
                ttlSeconds = ttl.toLong(),
            )
        }
        Unit
    }

    "delete" -> {
        SourceEngineHostRpc.storage("source-cache", "delete", args[0] as String)
        Unit
    }
    "deleteSourceVariables" -> {
        val source = args[0] as String
        SourceEngineHostRpc.storage(
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
        SourceEngineHostRpc.storage("cookies", "get", key)?.let { Cookie(url = key, cookie = it) }
    }
    "insert" -> {
        (args[0] as Array<*>).filterIsInstance<Cookie>().forEach { cookie ->
            SourceEngineHostRpc.storage("cookies", "put", cookie.url, cookie.cookie)
        }
        Unit
    }
    "delete" -> {
        SourceEngineHostRpc.storage("cookies", "delete", args[0] as String)
        Unit
    }
    "deleteOkHttp" -> {
        SourceEngineHostRpc.storage("cookies", "deleteContains", contains = "|")
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

private object HeadlessBookHelp : BookHelpAccessor {
    override fun saveContent(bookSource: io.legado.app.data.entities.BookSource, book: Book, bookChapter: BookChapter, content: String) {
        // content 操作由入口设置 needSave=false；此处拒绝所有章节写入，保持解析结果只读。
        error("Source-engine content requests must not persist chapters")
    }

    override fun getCoverPath(bookUrl: String): String =
        error("The parser does not manage cover files")

    override suspend fun clearCacheExtra() = Unit
}

private object HeadlessContentProcessor : ContentProcessorAccessor {
    override fun getTitleReplaceRules(book: Book): List<ReplaceRule> = emptyList()
    override fun getContent(book: Book, chapter: BookChapter, content: String, useReplace: Boolean): CharSequence = content
    override fun upReplaceRules() = Unit
}

private object HeadlessFileCache : FileCacheProvider {
    override fun put(key: String, value: String, saveTime: Int, persistent: Boolean) {
        SourceEngineHostRpc.storage(fileCacheNamespace(persistent), "put", key, value, saveTime.toLong())
    }

    override fun getAsString(key: String, persistent: Boolean): String? =
        SourceEngineHostRpc.storage(fileCacheNamespace(persistent), "get", key)

    override fun put(key: String, value: ByteArray, saveTime: Int, persistent: Boolean) {
        put(key, "base64:${value.toByteString().base64()}", saveTime, persistent)
    }

    override fun getAsBinary(key: String, persistent: Boolean): ByteArray? {
        val encoded = getAsString(key, persistent) ?: return null
        // 字符串缓存和二进制缓存共用 Rust 的字符串存储；显式前缀用于区分格式，
        // 避免把普通文本误当 Base64 解码后返回损坏数据。
        if (!encoded.startsWith("base64:")) return null
        return encoded.removePrefix("base64:").decodeBase64()?.toByteArray()
    }

    override fun remove(key: String, persistent: Boolean) {
        SourceEngineHostRpc.storage(fileCacheNamespace(persistent), "delete", key)
    }

    private fun fileCacheNamespace(persistent: Boolean) = if (persistent) "persistent-file-cache" else "file-cache"
}

private object HeadlessSourceCache : SourceCacheProvider {
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

private object HeadlessExploreKindsCache : ExploreKindsCacheProvider {
    override fun getAsString(key: String): String? = SourceEngineHostRpc.storage("explore-kinds", "get", key)
    override fun put(key: String, value: String) { SourceEngineHostRpc.storage("explore-kinds", "put", key, value) }
    override fun remove(key: String) { SourceEngineHostRpc.storage("explore-kinds", "delete", key) }
}

private object HeadlessRuleBigData : RuleBigDataProvider {
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
        SourceEngineHostRpc.storage("book-variables", if (value == null) "delete" else "put", key, value)
    }

    private fun get(key: String): String? = SourceEngineHostRpc.storage("book-variables", "get", key)

    private fun bigDataKey(vararg parts: String): String = parts.joinToString(":") {
        it.encodeToByteArray().toByteString().base64Url().trimEnd('=')
    }
}

private object HeadlessSourceNetwork : SourceNetworkProvider {
    override fun getCookie(tag: String): String = SharedCookieStore.getCookie(tag)
    override fun replaceCookie(tag: String, cookie: String) = SharedCookieStore.replaceCookie(tag, cookie)
    override fun removeCookie(tag: String) = SharedCookieStore.removeCookie(tag)
    override fun asBinding(): Any = SharedCookieStore
}

private object HeadlessSourceDebugLogger : SourceDebugLogger {
    override fun log(key: String, msg: String, print: Boolean, state: Int, showTime: Boolean) {
        if (!print) return
        val line = msg.replace('\n', ' ').take(4096)
        System.err.println("[source:$state] $key $line")
    }

    override fun log(msg: String) = log("", msg)
}
