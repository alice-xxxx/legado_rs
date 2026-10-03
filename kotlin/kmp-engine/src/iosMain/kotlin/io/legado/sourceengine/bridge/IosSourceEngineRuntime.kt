package io.legado.sourceengine.bridge

import io.legado.app.constant.AppConst
import io.legado.app.data.AppDbAccessor
import io.legado.app.data.AppDbProviders
import io.legado.app.data.dao.BookChapterDao
import io.legado.app.data.dao.BookDao
import io.legado.app.data.dao.BookGroupDao
import io.legado.app.data.dao.BookSourceDao
import io.legado.app.data.dao.BookmarkDao
import io.legado.app.data.dao.CacheDao
import io.legado.app.data.dao.CookieDao
import io.legado.app.data.dao.DictRuleDao
import io.legado.app.data.dao.HttpTTSDao
import io.legado.app.data.dao.KeyboardAssistsDao
import io.legado.app.data.dao.ReadRecordDao
import io.legado.app.data.dao.ReplaceRuleDao
import io.legado.app.data.dao.RuleSubDao
import io.legado.app.data.dao.SearchKeywordDao
import io.legado.app.data.dao.ServerDao
import io.legado.app.data.dao.SourceFilterRuleDao
import io.legado.app.data.dao.TxtTocRuleDao
import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.Cache
import io.legado.app.data.entities.Cookie
import io.legado.app.data.entities.ReplaceRule
import io.legado.app.help.ExploreKindsCacheProvider
import io.legado.app.help.ExploreKindsCacheProviders
import io.legado.app.help.CacheManager
import io.legado.app.help.FileCacheProvider
import io.legado.app.help.FileCacheProviders
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
import io.legado.app.model.script.registerNativeJsEngines
import io.legado.app.model.webBook.registerNativeWebBookProviders
import io.legado.app.utils.systemCurrentTimeMillis
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flowOf
import okio.ByteString.Companion.decodeBase64
import okio.ByteString.Companion.toByteString

/**
 * 独立 iOS 执行器的 provider 启动入口：不启动源平台 Room、偏好设置、文件缓存或 HTTP 客户端。
 * 跨请求状态通过 Rust HostStorage 保存；章节 DAO 只提供解析器所需的空行为，不连接书架数据库。
 */
object IosSourceEngineRuntime {
    private val installOnce = lazy(LazyThreadSafetyMode.SYNCHRONIZED) {
        PreferenceProviders.register(HeadlessPreferenceProvider)
        AppConfigProviders.register(HeadlessAppConfig)
        AppDbProviders.register(HeadlessAppDb)
        BookHelpProviders.register(HeadlessBookHelp)
        ContentProcessorProviders.register(HeadlessContentProcessor)
        FileCacheProviders.impl = HeadlessFileCache
        SourceCacheProviders.impl = HeadlessSourceCache
        ExploreKindsCacheProviders.impl = HeadlessExploreKindsCache
        RuleBigDataProviders.impl = HeadlessRuleBigData
        SourceDebugLoggers.impl = HeadlessSourceDebugLogger
        CookieStoreProviders.register(SharedCookieStore)
        registerSharedCookieJarBridge()
        SourceNetworkProviders.impl = HeadlessSourceNetwork
        UserAgentProviders.impl = UserAgentProvider { AppConst.DEFAULT_USER_AGENT }

        // 注册 Native 规则替换、URL intent 和目录刷新 provider，完成 framework 执行环境初始化。
        registerNativeWebBookProviders()
        registerNativeJsEngines(RustImageOpsProvider(::rustImageRequest))
        io.legado.app.help.http.registerNativeHttpProvider()
    }

    fun install() {
        installOnce.value
    }
}

private object HeadlessPreferenceProvider : PreferenceProvider {
    private val values = mutableMapOf<String, Any>()
    private val listeners = mutableSetOf<(String) -> Unit>()

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
        listeners.toList().forEach { it(key) }
    }
    override fun contains(key: String): Boolean = values.containsKey(key)
    override fun getAll(): Map<String, *> = values.toMap()
    override fun addPreferenceChangeListener(listener: (key: String) -> Unit): () -> Unit {
        listeners += listener
        return { listeners -= listener }
    }

    private fun put(key: String, value: Any?) {
        if (value == null) values.remove(key) else values[key] = value
        listeners.toList().forEach { it(key) }
    }
}

/** AppConfig is a UI contract; source execution receives explicit headless defaults. */
private object HeadlessAppConfig : AppConfigAccessor {
    override val threadCount = 16
    override val tocCountWords = false
    override fun setTocCountWords(value: Boolean) = Unit
    override val tocUiUseReplace = false
    override fun setTocUiUseReplace(value: Boolean) = Unit
    override var chineseConverterType = 0
    override val replaceEnableDefault = true
    override val enableReadRecord = false
    override val bookshelfSort = 0
    override val bookshelfLayout = 0
    override val bookshelfCoverHeight = 120
    override val bookshelfGridWidth = 120
    override val showUnread = true
    override val bookshelfListShowKind = false
    override val bookshelfListShowIntro = false
    override val bookshelfListIntroLines = 2
    override val bookshelfShowGroupCount = true
    override val bookshelfFixedWidthMode = false
    override val showLastUpdateTime = false
    override val saveTabPosition = 0
    override val bookExportFileName = ""
    override val episodeExportFileName = ""
    override val bookGroupStyle = 0
    override val autoRefreshBook = false
    override val preDownloadNum = 10
    override var changeSourceCheckAuthor = false
    override var changeSourceLoadInfo = false
    override var changeSourceLoadToc = false
    override var changeSourceLoadWordCount = false
    override var searchScope = ""
    override var searchGroup = ""
    override val searchLayout = 1
    override val precisionSearch = false
    override fun setPrecisionSearch(value: Boolean) = Unit
    override val exportCharset = "UTF-8"
    override val webDavUrl = ""
    override val webDavAccount = ""
    override val webDavPassword = ""
    override val syncBookProgress = false
    override val webDavDir = "legado"
    override val webDavDeviceName = ""
    override val ttsEngine = ""
    override val audioPlayUseWakeLock = false
    override val showAddToShelfAlert = true
    override fun setTtsEngine(value: String?) = Unit
    override val ttsSpeechRate = 5
    override val ttsTimer = 0
    override val themeMode = "0"
    override val isNightTheme = false
    override val isEInkMode = false
    override val systemNightTheme = false
    override val useDefaultCover = false
    override val coverDrawBookName = true
    override val coverDrawBookAuthor = true
    override val bottomBarHeight = 50
    override val bottomBarIconSize = 24
    override val bottomBarLabelMode = 0
    override val showHome = true
    override val showDiscovery = true
    override val bottomNavItemOrder = ""
    override val defaultHomePage = "bookshelf"
    override var importKeepName = false
    override var importKeepGroup = false
    override var importKeepEnable = false
    override val localBookImportSort = 0
    override val remoteServerId = 0L
    override val batchChangeSourceDelay = 0
    override val webPort = 1122
    override val bitmapCacheSize = 50
    override fun setBitmapCacheSize(value: Int) = Unit
    override val sourceEditMaxLine = Int.MAX_VALUE
    override val welcomeShowTime = 0
    override val devFeat = false
    override val bookInfoHorizontalLayout = false
}

private object HeadlessAppDb : AppDbAccessor {
    override val bookDao: BookDao get() = unavailableDao("bookDao")
    override val bookSourceDao: BookSourceDao get() = unavailableDao("bookSourceDao")
    override val bookChapterDao: BookChapterDao = HeadlessBookChapterDao
    override val bookGroupDao: BookGroupDao get() = unavailableDao("bookGroupDao")
    override val replaceRuleDao: ReplaceRuleDao get() = unavailableDao("replaceRuleDao")
    override val readRecordDao: ReadRecordDao get() = unavailableDao("readRecordDao")
    override val serverDao: ServerDao get() = unavailableDao("serverDao")
    override val txtTocRuleDao: TxtTocRuleDao get() = unavailableDao("txtTocRuleDao")
    override val dictRuleDao: DictRuleDao get() = unavailableDao("dictRuleDao")
    override val sourceFilterRuleDao: SourceFilterRuleDao get() = unavailableDao("sourceFilterRuleDao")
    override val httpTTSDao: HttpTTSDao get() = unavailableDao("httpTTSDao")
    override val cacheDao: CacheDao = HostCacheDao
    override val cookieDao: CookieDao = HostCookieDao
    override val ruleSubDao: RuleSubDao get() = unavailableDao("ruleSubDao")
    override val bookmarkDao: BookmarkDao get() = unavailableDao("bookmarkDao")
    override val searchKeywordDao: SearchKeywordDao get() = unavailableDao("searchKeywordDao")
    override val keyboardAssistsDao: KeyboardAssistsDao get() = unavailableDao("keyboardAssistsDao")
    override suspend fun <R> runInTransactionSuspending(block: suspend () -> R): R = block()
}

private fun <T> unavailableDao(name: String): T =
    error("The standalone source engine does not use app database DAO $name")

private object HeadlessBookChapterDao : BookChapterDao {
    override suspend fun search(bookUrl: String, key: String, start: Int, end: Int) = emptyList<BookChapter>()
    override suspend fun getChapterList(bookUrl: String) = emptyList<BookChapter>()
    override suspend fun getChapterList(bookUrl: String, start: Int, end: Int) = emptyList<BookChapter>()
    override suspend fun getChapter(bookUrl: String, index: Int): BookChapter? = null
    override fun flowChapter(bookUrl: String, index: Int): Flow<BookChapter?> = flowOf(null)
    override suspend fun getChapterCount(bookUrl: String) = 0
    override suspend fun insert(vararg bookChapter: BookChapter) = Unit
    override suspend fun update(vararg bookChapter: BookChapter) = Unit
    override suspend fun delByBook(bookUrl: String) = Unit
    override suspend fun upWordCount(bookUrl: String, url: String, wordCount: String) = Unit
    override suspend fun upEnd(bookUrl: String, url: String, end: Long?) = Unit
    override suspend fun upResourceUrl(bookUrl: String, url: String, resourceUrl: String?) = Unit
    override suspend fun upTitle(bookUrl: String, url: String, title: String) = Unit
    override suspend fun deleteNotShelfBookChapters() = Unit
}

private object HostCacheDao : CacheDao {
    override suspend fun get(key: String): Cache? =
        SourceEngineHostRpc.storage("source-cache", "get", key)?.let { encoded ->
            val split = encoded.indexOf('\n')
            check(split >= 0) { "Invalid cached source value for '$key'" }
            Cache(key = key, deadline = encoded.substring(0, split).toLong(), value = encoded.substring(split + 1))
        }

    override suspend fun insert(vararg cache: Cache) {
        cache.forEach { item ->
            val ttl = if (item.deadline <= 0L) 0L else
                ((item.deadline - currentTimeMillis()) / 1000L).coerceAtLeast(1L)
            SourceEngineHostRpc.storage(
                "source-cache", "put", item.key, "${item.deadline}\n${item.value.orEmpty()}", ttl,
            )
        }
    }

    override suspend fun delete(key: String) {
        SourceEngineHostRpc.storage("source-cache", "delete", key)
    }

    override suspend fun deleteSourceVariables(key: String) {
        SourceEngineHostRpc.storage(
            "source-cache", "deletePrefixes",
            keyPrefixes = listOf("v_${key}_", "userInfo_$key", "loginHeader_$key", "sourceVariable_$key"),
        )
    }

    override suspend fun clearDeadline(now: Long) = Unit
}

private object HostCookieDao : CookieDao {
    override suspend fun get(url: String): Cookie? =
        SourceEngineHostRpc.storage("cookies", "get", url)?.let { Cookie(url = url, cookie = it) }

    override suspend fun insert(vararg cookie: Cookie) {
        cookie.forEach { SourceEngineHostRpc.storage("cookies", "put", it.url, it.cookie) }
    }

    override suspend fun delete(url: String) {
        SourceEngineHostRpc.storage("cookies", "delete", url)
    }

    override suspend fun deleteOkHttp() {
        SourceEngineHostRpc.storage("cookies", "deleteContains", contains = "|")
    }
}

private object HeadlessBookHelp : BookHelpAccessor {
    override fun saveContent(source: io.legado.app.data.entities.BookSource, book: Book, chapter: BookChapter, content: String) {
        error("Source-engine content requests must not persist chapters")
    }

    override fun getCoverPath(bookUrl: String): String = error("The parser does not manage cover files")
    override suspend fun clearCacheExtra() = Unit
}

private object HeadlessContentProcessor : ContentProcessorAccessor {
    override fun getTitleReplaceRules(book: Book): List<ReplaceRule> = emptyList()
    override fun getContent(book: Book, chapter: BookChapter, content: String, useReplace: Boolean): CharSequence = content
    override fun upReplaceRules() = Unit
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

private object HeadlessFileCache : FileCacheProvider {
    override fun put(key: String, value: String, saveTime: Int, persistent: Boolean) {
        SourceEngineHostRpc.storage(cacheNamespace(persistent), "put", key, value, saveTime.toLong())
    }
    override fun getAsString(key: String, persistent: Boolean): String? =
        SourceEngineHostRpc.storage(cacheNamespace(persistent), "get", key)
    override fun put(key: String, value: ByteArray, saveTime: Int, persistent: Boolean) {
        put(key, "base64:${value.toByteString().base64()}", saveTime, persistent)
    }
    override fun getAsBinary(key: String, persistent: Boolean): ByteArray? =
        getAsString(key, persistent)?.takeIf { it.startsWith("base64:") }
            ?.removePrefix("base64:")?.decodeBase64()?.toByteArray()
    override fun remove(key: String, persistent: Boolean) {
        SourceEngineHostRpc.storage(cacheNamespace(persistent), "delete", key)
    }
    private fun cacheNamespace(persistent: Boolean) = if (persistent) "persistent-file-cache" else "file-cache"
}

private object HeadlessExploreKindsCache : ExploreKindsCacheProvider {
    override fun getAsString(key: String): String? = SourceEngineHostRpc.storage("explore-kinds", "get", key)
    override fun put(key: String, value: String) { SourceEngineHostRpc.storage("explore-kinds", "put", key, value) }
    override fun remove(key: String) { SourceEngineHostRpc.storage("explore-kinds", "delete", key) }
}

private object HeadlessRuleBigData : RuleBigDataProvider {
    override fun putBookVariable(bookUrl: String, key: String, value: String?) = put(storageKey(bookUrl, key), value)
    override fun getBookVariable(bookUrl: String, key: String?): String? = get(storageKey(bookUrl, key.orEmpty()))
    override fun hasBookVariable(bookUrl: String, key: String): Boolean = get(storageKey(bookUrl, key)) != null
    override fun putChapterVariable(bookUrl: String, chapterUrl: String, key: String, value: String?) =
        put(storageKey(bookUrl, chapterUrl, key), value)
    override fun getChapterVariable(bookUrl: String, chapterUrl: String, key: String): String? =
        get(storageKey(bookUrl, chapterUrl, key))
    override fun listBookDataDirs(): List<String> = emptyList()
    override fun clearInvalidBookData(bookUrls: Set<String>) = Unit
    private fun put(key: String, value: String?) {
        SourceEngineHostRpc.storage("book-variables", if (value == null) "delete" else "put", key, value)
    }
    private fun get(key: String): String? = SourceEngineHostRpc.storage("book-variables", "get", key)
    private fun storageKey(vararg parts: String) = parts.joinToString(":") {
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
        if (print) println("[source:$state] $key ${msg.replace('\n', ' ').take(4096)}")
    }
    override fun log(msg: String) = log("", msg)
}


private fun currentTimeMillis(): Long = systemCurrentTimeMillis()
