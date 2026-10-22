package io.legado.sourceengine.bridge

import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BaseSource
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.HttpTTS
import io.legado.app.data.entities.OldRssSource
import io.legado.app.data.entities.SearchBook
import io.legado.app.data.entities.rule.RowUi
import io.legado.app.data.entities.rule.ExploreKind
import io.legado.app.data.entities.toBookSource
import io.legado.app.help.http.header
import io.legado.app.help.http.CookieStoreProviders
import io.legado.app.help.tts.HttpTtsRequest
import io.legado.app.help.source.SourceCacheProviders
import io.legado.app.help.source.VerificationUiProvider
import io.legado.app.help.source.VerificationUiProviders
import io.legado.app.help.source.SourceVerificationHelpShared
import io.legado.app.help.toast.Toaster
import io.legado.app.help.toast.Toasters
import io.legado.app.help.ui.OpenUrlProvider
import io.legado.app.help.ui.OpenUrlProviders
import io.legado.app.help.ui.JsUiProvider
import io.legado.app.help.ui.JsUiProviders
import io.legado.app.help.source.exploreKinds
import io.legado.app.model.analyzeRule.AnalyzeUrlFactories
import io.legado.app.model.webBook.WebBook
import io.legado.app.utils.KS_JSON
import io.legado.app.utils.NetworkUtils
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.decodeFromJsonElement
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

/**
 * 把 Rust 请求映射到共享的 WebBook 编排入口。
 *
 * 解析顺序和字段语义继续由 WebBook / AnalyzeUrlCore 决定；这里仅负责稳定的跨语言参数与结果 JSON。
 * 宿主跟随本次协程传递，避免并发请求之间覆盖全局 HTTP 回调目标。
 */
@OptIn(ExperimentalEncodingApi::class)
object WebBookSourceRuleExecutor : SourceRuleExecutor {
    override suspend fun execute(call: SourceEngineCall, host: SourceEngineHost): String =
        executeWithHost(call, host.forExecution(call.executionId))

    private suspend fun executeWithHost(call: SourceEngineCall, host: SourceEngineHost): String =
        SourceEngineHostRegistry.withHost(host) {
            withContext(SourceEngineHostContext(host)) {
                when (call.operation) {
                    "login" -> sourceLogin(call, host)

                    "search" -> {
                        kotlinx.coroutines.withTimeout(10_000L) {
                            encodeSearchPage(decodeBookSource(call), call.keyword.orEmpty(), call.page ?: 1)
                                .toString()
                        }
                    }

                    "exploreKinds" -> encodeExploreKinds(decodeBookSource(call))

                    "explore" -> encodeBookList(
                        WebBook.getBookListAwait(
                            bookSource = decodeBookSource(call),
                            key = call.keyword ?: error("Missing category URL for explore"),
                            page = call.page ?: 1,
                            isSearch = false,
                        ),
                    )

                    "rssExploreKinds" -> encodeExploreKinds(decodeRssSource(call))

                    "rssExplore" -> encodeBookList(
                        WebBook.getBookListAwait(
                            bookSource = decodeRssSource(call),
                            key = call.keyword ?: error("Missing category URL for RSS explore"),
                            page = call.page ?: 1,
                            isSearch = false,
                        ),
                    )

                    "rssBookInfo" -> {
                        val book = decodeBook(call)
                        KS_JSON.encodeToString(
                            Book.serializer(),
                            WebBook.getBookInfoAwait(decodeRssSource(call), book),
                        )
                    }

                    "rssChapters" -> {
                        val book = decodeBook(call)
                        val chapters = WebBook.getChapterListAwait(decodeRssSource(call), book).getOrThrow()
                        KS_JSON.encodeToString(ListSerializer(BookChapter.serializer()), chapters)
                    }

                    "rssContent" -> {
                        val book = decodeBook(call)
                        val chapter = decodeChapter(call)
                        JsonPrimitive(
                            WebBook.getContentAwait(
                                bookSource = decodeRssSource(call),
                                book = book,
                                bookChapter = chapter,
                                nextChapterUrl = call.nextChapterUrl,
                                needSave = false,
                            ),
                        ).toString()
                    }

                    "bookInfo" -> {
                        val book = decodeBook(call)
                        val source = decodeBookSource(call)
                        KS_JSON.encodeToString(Book.serializer(), WebBook.getBookInfoAwait(source, book))
                    }

                    "chapters" -> {
                        val book = decodeBook(call)
                        val source = decodeBookSource(call)
                        val chapters = WebBook.getChapterListAwait(source, book).getOrThrow()
                        KS_JSON.encodeToString(ListSerializer(BookChapter.serializer()), chapters)
                    }

                    "content" -> {
                        val book = decodeBook(call)
                        val chapter = decodeChapter(call)
                        val source = decodeBookSource(call)
                        JsonPrimitive(
                            WebBook.getContentAwait(
                                bookSource = source,
                                book = book,
                                bookChapter = chapter,
                                nextChapterUrl = call.nextChapterUrl,
                                // 此接口只返回章节正文；是否写入书架由上层阅读器决定。
                                needSave = false,
                            ),
                        ).toString()
                    }

                    "resolveMedia" -> {
                        val book = decodeBook(call)
                        val chapter = decodeChapter(call)
                        val source = decodeBookSource(call)
                        val (url, headers) = AnalyzeUrlFactories.create(
                            rawUrl = call.mediaUrl ?: error("Missing media URL"),
                            source = source,
                            ruleData = book,
                            chapter = chapter,
                            coroutineContext = currentCoroutineContext(),
                            headerMapF = call.mediaHeaders,
                        ).resolveMedia()
                        buildJsonObject {
                            put("url", url)
                            put("headers", buildJsonObject {
                                headers.forEach { (name, value) -> put(name, value) }
                            })
                        }.toString()
                    }

                    "httpTtsAudio" -> {
                        val config = KS_JSON.decodeFromJsonElement(
                            HttpTTS.serializer(),
                            KS_JSON.parseToJsonElement(call.sourceJson),
                        )
                        val text = call.keyword ?: error("Missing HTTP TTS text")
                        check(text.isNotBlank() && text.length <= 20_000 && text.none {
                            it == '\u0000' || (it.isISOControl() && it !in "\n\r\t")
                        }) {
                            "HTTP TTS text must contain 1–20000 characters without invalid control characters"
                        }
                        val speechRate = call.page ?: error("Missing HTTP TTS speech rate")
                        check(speechRate in 0..45) { "HTTP TTS speech rate must be between 0 and 45" }
                        val response = HttpTtsRequest.audioResponse(
                            config = config,
                            text = text,
                            speechRate = speechRate,
                            context = currentCoroutineContext(),
                        )
                        try {
                            val declaredLength = response.header("Content-Length")?.toLongOrNull()
                            check(declaredLength == null || declaredLength <= MAX_HTTP_TTS_AUDIO_BYTES) {
                                "HTTP TTS audio exceeds the 16 MiB limit"
                            }
                            val bytes = response.body.bytes()
                            check(bytes.isNotEmpty()) { "HTTP TTS returned an empty audio response" }
                            check(bytes.size <= MAX_HTTP_TTS_AUDIO_BYTES) {
                                "HTTP TTS audio exceeds the 16 MiB limit"
                            }
                            val contentType = response.header("Content-Type")
                                ?.substringBefore(';')
                                ?.trim()
                                .orEmpty()
                            buildJsonObject {
                                put("bytesBase64", Base64.encode(bytes))
                                put("contentType", contentType)
                            }.toString()
                        } finally {
                            response.close()
                        }
                    }

                    else -> error("Unsupported source-engine operation: ${call.operation}")
                }
            }
        }

    private fun decodeBookSource(call: SourceEngineCall): BookSource {
        val sourceJson = KS_JSON.parseToJsonElement(call.sourceJson)
        return decodeBookSource(sourceJson)
    }

    private fun decodeBookSource(sourceJson: JsonElement): BookSource =
        KS_JSON.decodeFromJsonElement(BookSource.serializer(), sourceJson).apply {
            sourceUserAgentOverride = sourceUserAgentOverride(sourceJson)
        }

    private suspend fun encodeSearchPage(source: BookSource, keyword: String, pageNumber: Int) =
        WebBook.getBookListAwait(
            bookSource = source,
            key = keyword,
            page = pageNumber,
        ).let { page ->
            buildJsonObject {
                put(
                    "books",
                    JsonArray(page.books.map {
                        KS_JSON.encodeToJsonElement(SearchBook.serializer(), it)
                    }),
                )
                put("hasNextPage", page.hasNextPage)
            }
        }

    private const val MAX_HTTP_TTS_AUDIO_BYTES = 16 * 1024 * 1024

    private fun decodeRssSource(call: SourceEngineCall): BookSource {
        val sourceJson = KS_JSON.parseToJsonElement(call.sourceJson)
        return KS_JSON.decodeFromJsonElement(OldRssSource.serializer(), sourceJson).toBookSource().apply {
            sourceUserAgentOverride = sourceUserAgentOverride(sourceJson)
        }
    }

    private fun sourceUserAgentOverride(sourceJson: JsonElement): String? {
        val value = (sourceJson as? JsonObject)?.get("legadoRsUserAgentOverride") as? JsonPrimitive
            ?: return null
        if (!value.isString) return null
        val userAgent = value.content.trim()
        if (userAgent.isEmpty() || userAgent.encodeToByteArray().size > 512) return null
        if (userAgent.any { it.code in 0..31 || it.code in 127..159 }) return null
        return userAgent
    }

    private fun decodeBook(call: SourceEngineCall): Book =
        KS_JSON.decodeFromString(
            Book.serializer(),
            call.bookJson ?: error("Missing book JSON for ${call.operation}"),
        )

    private fun decodeChapter(call: SourceEngineCall): BookChapter =
        KS_JSON.decodeFromString(
            BookChapter.serializer(),
            call.chapterJson ?: error("Missing chapter JSON for ${call.operation}"),
        )

    /** 登录统一执行后读取状态；表单刷新和输入回填在同一次调用内完成。 */
    private suspend fun sourceLogin(call: SourceEngineCall, host: SourceEngineHost): String {
        val source = decodeBookSource(call)
        val loginUrl = source.getLoginJs()?.trim().orEmpty()
        val web = loginUrl.isAbsoluteWebUrl()
        val result = when (call.keyword) {
            "open" -> "{}"
            "submit" -> if (web) "{}" else withSourceLoginUi { submitLogin(source, call) }
            "cookies" -> importSourceLoginCookies(source, call.credentials?.get("loginUrl"), call.credentials)
            "clear" -> {
                val store = CookieStoreProviders.get() ?: error("Source cookie store is not installed")
                // 同时移除 Cookie、登录头和表单数据；存储失败必须向上传递。
                store.removeCookieStrict(source.getKey())
                source.removeLoginHeader()
                source.removeLoginInfo()
                buildJsonObject { put("message", "已清除登录数据。") }.toString()
            }
            else -> error("Unknown login action")
        }.let { KS_JSON.parseToJsonElement(it) as JsonObject }
        currentCoroutineContext().ensureActive()
        val sourceUrl = source.getKey()
        val scope = NetworkUtils.getSubDomain(sourceUrl)
        val cache = SourceCacheProviders.impl ?: error("Source cache is not installed")
        val cookies = CookieStoreProviders.get()
        // 这里只表示有已保存的数据，不表示已通过网站认证。
        val saved = !cache.get("loginHeader_$sourceUrl").isNullOrBlank()
            || !cache.get("userInfo_$sourceUrl").isNullOrBlank()
            || !cookies?.getCookieNoSession(sourceUrl).isNullOrBlank()
            || !cookies?.getSessionCookie(scope).isNullOrBlank()
            || host.storage.hasCookieJarCookies(scope, sourceUrl)
        return buildJsonObject {
            result.forEach { (key, value) -> put(key, value) }
            put("hasLoginState", saved)
            if (call.keyword == "open" || (result["refreshTargets"] as? JsonArray)?.contains(JsonPrimitive("login")) == true) {
                put("form", KS_JSON.parseToJsonElement(encodeSourceLoginForm(source)))
            }
            // 原生窗口能力只传给 Rust，不能透给 Vue。
            if (web && call.keyword in setOf("open", "submit")) {
                put("web", buildJsonObject {
                    put("url", loginUrl)
                    put("cookieHeader", cookies?.getCookie(loginUrl).orEmpty())
                })
            }
        }.toString()
    }

    /** 普通登录和表单按钮共享输入校验、错误处理及回填，只在执行脚本处区分。 */
    private suspend fun submitLogin(source: BookSource, call: SourceEngineCall): String = try {
        val rows = source.loginUi().orEmpty()
        val credentials = requireNotNull(validateLoginInputs(source, rows, call.credentials)) {
            "登录输入与此书源表单不匹配。"
        }
        val script = source.getLoginJs().orEmpty()
        require(!script.trim().isAbsoluteWebUrl()) { "此书源使用网页登录。" }
        if (call.actionId == null) {
            require(script.isNotBlank()) { "此书源没有可执行的登录规则。" }
            if (rows.any { it.type in supportedLoginFieldTypes }) {
                check(SourceCacheProviders.impl != null) { "登录存储不可用。" }
                val info = buildJsonObject { credentials.forEach { (name, value) -> put(name, value) } }
                check(source.putLoginInfo(info.toString())) { "登录信息无法保存。" }
            }
            source.login()
        } else {
            val button = requireNotNull(rows.getOrNull(call.actionId)?.takeIf {
                it.type == RowUi.Type.button && !it.action.isNullOrBlank() && !it.action.orEmpty().isAbsoluteWebUrl()
            }) { "此书源登录操作不可用。" }
            source.evalJS("$script\n${button.action}") { put("result", HashMap(credentials)) }
        }
        currentCoroutineContext().ensureActive()
        buildJsonObject {
            put("message", "登录操作已执行。")
            put("inputUpdates", buildJsonObject {
                rows.filter { it.type == RowUi.Type.password }.forEach { put(it.name, "") }
            })
        }.toString()
    } catch (error: Throwable) {
        currentCoroutineContext().ensureActive()
        buildJsonObject { put("error", error.message?.take(4096) ?: "登录操作失败。") }.toString()
    }

    private fun encodeSourceLoginForm(source: BookSource): String {
        val loginJs = source.getLoginJs()?.trim().orEmpty()
        if (loginJs.isAbsoluteWebUrl()) {
            return buildJsonObject { put("mode", "web") }.toString()
        }
        val canLogin = loginJs.isNotEmpty()
        val rows = source.loginUi().orEmpty()
        val actions = rows.mapIndexedNotNull { index, row ->
            row.takeIf {
                it.type == RowUi.Type.button && !it.action.isNullOrBlank()
                    && !it.action.orEmpty().isAbsoluteWebUrl()
            }?.let {
                buildJsonObject {
                    put("id", index)
                    put("label", it.name)
                }
            }
        }
        val fields = rows.mapNotNull { row ->
            val type = when (row.type) {
                RowUi.Type.text -> "text"
                RowUi.Type.password -> "password"
                RowUi.Type.select -> "select"
                RowUi.Type.toggle -> "toggle"
                else -> return@mapNotNull null
            }
            buildJsonObject {
                put("name", row.name)
                put("label", row.name)
                put("type", type)
                put("password", type == "password")
                if (type == "select") {
                    put("choices", JsonArray(row.chars.orEmpty().map(::JsonPrimitive)))
                }
            }
        }

        require(canLogin || actions.isNotEmpty()) { "此书源没有可用的登录表单。" }
        require(fields.map { it.getValue("name") }.let { it.size == it.toSet().size }) {
            "此书源登录表单包含重复字段名称。"
        }
        val actionIds = actions.map { it.getValue("id").jsonPrimitive.int }.toSet()
        val layout = rows.mapIndexedNotNull { index, row ->
            val kind = when {
                row.type in supportedLoginFieldTypes -> "field"
                row.type == RowUi.Type.button && index in actionIds -> "action"
                row.type == RowUi.Type.title && row.name.isNotBlank() -> "title"
                else -> return@mapIndexedNotNull null
            }
            val defaultCols = if (kind == "action") 2 else 1
            val basis = row.style?.layout_flexBasisPercent ?: 0F
            val legacyCols = when {
                !basis.isFinite() || basis <= 0F -> null
                basis >= 0.75F -> 1
                basis >= 0.42F -> 2
                basis >= 0.29F -> 3
                else -> 4
            }
            buildJsonObject {
                put("kind", kind)
                when (kind) {
                    "field" -> put("name", row.name)
                    "action" -> put("id", index)
                    "title" -> put("label", row.name)
                }
                put("cols", (row.style?.cols ?: legacyCols ?: defaultCols).coerceIn(1, 4))
                put("rows", (row.style?.rows ?: 1).coerceIn(1, 12))
            }
        }
        return buildJsonObject {
            put("mode", "form")
            put("canLogin", canLogin)
            put("fields", JsonArray(fields))
            put("actions", JsonArray(actions))
            put("layout", JsonArray(layout))
        }.toString()
    }

    /** Import only cookies returned by the native private browser for this source's login URL. */
    private fun importSourceLoginCookies(
        source: BookSource,
        cookieUrl: String?,
        input: Map<String, String>?,
    ): String {
        val loginUrl = source.getLoginJs()?.trim().orEmpty()
        check(loginUrl.isAbsoluteWebUrl() && loginUrl == cookieUrl) {
            "Source login URL changed while the private browser was open"
        }
        val cookieHeader = input?.get("cookieHeader").orEmpty()
        check(cookieHeader.encodeToByteArray().size <= 256 * 1024) {
            "Private browser cookies exceed the supported size"
        }
        check(cookieHeader.none { it == '\r' || it == '\n' }) {
            "Private browser cookies contain invalid characters"
        }
        if (cookieHeader.isNotBlank()) {
            val cookieStore = CookieStoreProviders.get()
                ?: error("Source cookie storage is not installed")
            cookieStore.replaceCookie(loginUrl, cookieHeader)
        }
        val importedCount = cookieHeader.split(';').count { it.substringBefore('=').isNotBlank() }
        return buildJsonObject {
            put("message", if (importedCount == 0) "网页登录已结束，未发现可保存的 Cookie。" else "网页登录 Cookie 已保存。")
        }.toString()
    }

    /** 注册原版 JS 使用的 UI 能力，浏览器等待及 Cookie 交给各平台宿主。 */
    private suspend fun withSourceLoginUi(block: suspend () -> String): String {
        val messages = mutableListOf<String>()
        val updates = mutableMapOf<String, String>()
        val refreshTargets = mutableSetOf<String>()
        var copyText: String? = null
        val previous = VerificationUiProviders.current()
        val previousToast = Toasters.current()
        val previousOpenUrl = OpenUrlProviders.current()
        val previousUi = JsUiProviders.impl
        Toasters.register(object : Toaster {
            override fun toast(message: String) { if (messages.size < 40) messages += message.take(4096) }
            override fun toastLong(message: String) = toast(message)
        })
        JsUiProviders.impl = object : JsUiProvider {
            override fun refresh(target: String) { if (target in setOf("login", "explore", "book")) refreshTargets += target }
            override fun updateLoginData(data: Map<String, String>) { updates.putAll(data) }
            override fun copy(text: String) { copyText = text }
        }
        fun openBrowser(source: BaseSource?, url: String, title: String, wait: Boolean): HostBrowserResponse = runBlocking {
            val host = requireNotNull(SourceEngineHostRegistry.current())
            val cookieStore = CookieStoreProviders.get()
            val cookie = if (url.isAbsoluteWebUrl()) cookieStore?.getCookie(url).orEmpty() else ""
            val headers = source?.getHeaderMap(true).orEmpty()
                .filterKeys { it.equals("User-Agent", true) }
                .map { (name, value) -> HostHeader(name, value) }
                .toMutableList()
            if (cookie.isNotEmpty()) headers += HostHeader("Cookie", cookie)
            val response = host.executeBrowser(HostBrowserRequest(
                capabilityId = SourceBrowserSnapshot.newCapabilityId(), html = "", baseUrl = url,
                javaScript = "", mode = HostBrowserMode.INTERACTIVE_PAGE, headers = headers,
                title = title.take(256), waitForResult = wait,
            ))
            if (response.cookieHeader.isNotEmpty() && response.finalUrl.isAbsoluteWebUrl()) {
                cookieStore?.replaceCookie(response.finalUrl, response.cookieHeader)
            }
            response
        }
        OpenUrlProviders.register(object : OpenUrlProvider {
            override fun openUrl(url: String, mimeType: String?, sourceKey: String?, sourceTag: String?, sourceType: Int) {
                openBrowser(null, url, sourceTag.orEmpty(), false)
            }
        })
        VerificationUiProviders.register(object : VerificationUiProvider {
            override fun awaitBrowser(source: BaseSource, url: String, title: String, refetchAfterSuccess: Boolean): io.legado.app.help.http.StrResponse {
                val response = openBrowser(source, url, title, true)
                val body = if (refetchAfterSuccess && response.finalUrl.isAbsoluteWebUrl()) {
                    runBlocking { AnalyzeUrlFactories.create(response.finalUrl, source = source).getStrResponseAwait().body }
                } else response.body
                return io.legado.app.help.http.StrResponse(response.finalUrl, body)
            }
            override fun showVerificationCodeDialog(url: String, source: BaseSource) {
                error("Image verification requires a verification input UI")
            }

            override fun startBrowser(
                source: BaseSource,
                url: String,
                title: String,
                saveResult: Boolean?,
                refetchAfterSuccess: Boolean?,
                asBottomSheet: Boolean,
            ) {
                val response = openBrowser(source, url, title, saveResult == true)
                if (saveResult == true) {
                    val body = if (refetchAfterSuccess == true && response.finalUrl.isAbsoluteWebUrl()) {
                        runBlocking { AnalyzeUrlFactories.create(response.finalUrl, source = source).getStrResponseAwait().body }
                    } else response.body
                    SourceVerificationHelpShared.setResult(source.getKey(), body)
                    SourceVerificationHelpShared.notifyResultArrived(source.getKey())
                }
            }
        })
        return try {
            val result = KS_JSON.parseToJsonElement(block()) as JsonObject
            buildJsonObject {
                result.forEach { (key, value) -> put(key, value) }
                if (messages.isNotEmpty()) put("messages", JsonArray(messages.map(::JsonPrimitive)))
                if (updates.isNotEmpty()) put("inputUpdates", buildJsonObject {
                    (result["inputUpdates"] as? JsonObject)?.forEach { (key, value) -> put(key, value) }
                    updates.forEach { (key, value) -> put(key, value) }
                })
                if (refreshTargets.isNotEmpty()) put("refreshTargets", JsonArray(refreshTargets.map(::JsonPrimitive)))
                copyText?.let { put("copyText", it) }
            }.toString()
        } finally {
            VerificationUiProviders.register(previous)
            Toasters.register(previousToast)
            OpenUrlProviders.register(previousOpenUrl)
            JsUiProviders.impl = previousUi
        }
    }

    private fun validateLoginInputs(
        source: BookSource,
        rows: List<RowUi>,
        input: Map<String, String>?,
    ): Map<String, String>? {
        val fields = rows.filter { it.type in supportedLoginFieldTypes }
        if (fields.map { it.name }.toSet().size != fields.size) return null
        val fieldNames = fields.mapTo(mutableSetOf()) { it.name }
        if (input.orEmpty().keys.any { it !in fieldNames }) return null
        val existing = source.getLoginInfoMap().orEmpty()
        return fields.associate { row ->
            val value = input?.get(row.name) ?: when (row.type) {
                RowUi.Type.select -> row.chars.orEmpty().firstOrNull().orEmpty()
                RowUi.Type.toggle -> "false"
                else -> ""
            }
            val resolved = when (row.type) {
                RowUi.Type.password -> value.ifEmpty { existing[row.name].orEmpty() }
                RowUi.Type.select -> value.takeIf { it in row.chars.orEmpty() } ?: return null
                RowUi.Type.toggle -> value.takeIf { it == "true" || it == "false" } ?: return null
                else -> value
            }
            row.name to resolved
        }
    }

    private fun String.isAbsoluteWebUrl(): Boolean =
        startsWith("http://", ignoreCase = true) || startsWith("https://", ignoreCase = true)

    private val supportedLoginFieldTypes = setOf(
        RowUi.Type.text,
        RowUi.Type.password,
        RowUi.Type.select,
        RowUi.Type.toggle,
    )

    private suspend fun encodeExploreKinds(source: BookSource): String =
        KS_JSON.encodeToString(ListSerializer(ExploreKind.serializer()), source.exploreKinds())

    private fun encodeBookList(page: io.legado.app.data.entities.BookListPage): String =
        buildJsonObject {
            put(
                "books",
                JsonArray(page.books.map {
                    KS_JSON.encodeToJsonElement(SearchBook.serializer(), it)
                }),
            )
            put("hasNextPage", page.hasNextPage)
        }.toString()
}
