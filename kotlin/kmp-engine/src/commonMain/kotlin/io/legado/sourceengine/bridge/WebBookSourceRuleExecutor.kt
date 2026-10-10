package io.legado.sourceengine.bridge

import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.HttpTTS
import io.legado.app.data.entities.OldRssSource
import io.legado.app.data.entities.SearchBook
import io.legado.app.data.entities.rule.RowUi
import io.legado.app.data.entities.rule.ExploreKind
import io.legado.app.data.entities.toBookSource
import io.legado.app.constant.AppConst
import io.legado.app.help.http.header
import io.legado.app.help.http.CookieStoreProviders
import io.legado.app.help.tts.HttpTtsRequest
import io.legado.app.help.source.SourceCacheProviders
import io.legado.app.help.source.exploreKinds
import io.legado.app.help.config.AppConfigProviders
import io.legado.app.help.coroutine.IoDispatcher
import io.legado.app.model.analyzeRule.AnalyzeUrlFactories
import io.legado.app.model.webBook.WebBook
import io.legado.app.utils.KS_JSON
import io.legado.app.utils.NetworkUtils
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.decodeFromJsonElement
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi
import kotlin.math.min

/**
 * 把 Rust 请求映射到共享的 WebBook 编排入口。
 *
 * 解析顺序和字段语义继续由 WebBook / AnalyzeUrlCore 决定；这里仅负责稳定的跨语言参数与结果 JSON。
 * 宿主跟随本次协程传递，避免并发请求之间覆盖全局 HTTP 回调目标。
 */
@OptIn(ExperimentalEncodingApi::class)
object WebBookSourceRuleExecutor : SourceRuleExecutor {
    override suspend fun execute(call: SourceEngineCall, host: SourceEngineHost): String =
        SourceEngineHostRegistry.withHost(host) {
            withContext(SourceEngineHostContext(host)) {
                when (call.operation) {
                    "sourceLoginForm" -> encodeSourceLoginForm(decodeBookSource(call))

                    "sourceLoginWebInfo" -> encodeSourceLoginWebInfo(decodeBookSource(call))

                    "sourceLoginImportCookies" -> importSourceLoginCookies(
                        decodeBookSource(call),
                        call.keyword,
                        call.credentials,
                    )

                    "loginSource" -> loginSource(decodeBookSource(call), call.credentials)

                    "sourceLoginAction" -> runSourceLoginAction(
                        decodeBookSource(call),
                        call.actionId,
                        call.credentials,
                    )

                    "sourceLoginState" -> {
                        val source = decodeBookSource(call)
                        val sourceUrl = source.getKey()
                        val cookieScope = NetworkUtils.getSubDomain(sourceUrl)
                        val sourceCache = SourceCacheProviders.impl
                            ?: error("Source cache is not installed")
                        val hasLoginHeader = sourceCache.get("loginHeader_$sourceUrl")
                            ?.isNotBlank() == true
                        val hasLoginInfo = sourceCache.get("userInfo_$sourceUrl")
                            ?.isNotBlank() == true
                        val cookieStore = CookieStoreProviders.get()
                        val hasStoredCookie = cookieStore?.getCookieNoSession(sourceUrl)
                            ?.isNotBlank() == true || cookieStore?.getSessionCookie(cookieScope)
                            ?.isNotBlank() == true
                        val hasRustCookie = host.storage.hasCookieJarCookies(cookieScope, sourceUrl)
                        buildJsonObject {
                            put("hasLoginState", hasLoginHeader || hasLoginInfo || hasStoredCookie || hasRustCookie)
                        }.toString()
                    }

                    "clearSourceLoginState" -> {
                        val source = decodeBookSource(call)
                        val sourceUrl = source.getKey()
                        val cookieStore = CookieStoreProviders.get()
                            ?: error("Source cookie store is not installed")
                        // Strict removal propagates private storage or Rust Jar failures to the UI.
                        cookieStore.removeCookieStrict(sourceUrl)
                        source.removeLoginHeader()
                        source.removeLoginInfo()
                        buildJsonObject { put("cleared", true) }.toString()
                    }

                    "search" -> {
                        encodeSearchPage(decodeBookSource(call), call.keyword.orEmpty(), call.page ?: 1)
                            .toString()
                    }

                    "searchBatch" -> searchSourcesBatch(call)

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

    private suspend fun searchSourcesBatch(call: SourceEngineCall): String {
        val sourceJson = KS_JSON.parseToJsonElement(call.sourceJson) as? JsonArray
            ?: error("Search batch sources must be an array")
        check(sourceJson.isNotEmpty()) { "Search batch cannot be empty" }
        val sources = sourceJson
        val concurrency = min(AppConfigProviders.get().threadCount, AppConst.MAX_THREAD).coerceAtLeast(1)
        // Keep KMP rule/JS execution on one lane; each source suspends into the host for HTTP, so
        // these requests still overlap while native QuickJS's process-wide scope slot stays safe.
        val dispatcher = IoDispatcher.limitedParallelism(1)
        val permits = Semaphore(concurrency)
        val results = coroutineScope {
            sources.map { sourceJson ->
                async(dispatcher) {
                    permits.withPermit {
                        try {
                            withTimeoutOrNull(AppConst.timeLimit) {
                                encodeSearchPage(
                                    decodeBookSource(sourceJson),
                                    call.keyword.orEmpty(),
                                    call.page ?: 1,
                                )
                            } ?: buildJsonObject { put("error", "Source search timed out") }
                        } catch (cancelled: CancellationException) {
                            throw cancelled
                        } catch (error: Exception) {
                            currentCoroutineContext().ensureActive()
                            buildJsonObject { put("error", error.message ?: error.toString()) }
                        }
                    }
                }
            }.awaitAll()
        }
        return buildJsonObject { put("results", JsonArray(results)) }.toString()
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

    private fun encodeSourceLoginForm(source: BookSource): String {
        val loginJs = source.getLoginJs()?.trim().orEmpty()
        if (loginJs.isAbsoluteWebUrl()) {
            return buildJsonObject {
                put("mode", "web")
                put("message", "此书源使用网页登录。")
            }.toString()
        }
        val canLogin = loginJs.isNotEmpty() && !loginJs.isAbsoluteWebUrl()
        val rows = source.loginUi().orEmpty()
        val actions = if (loginJs.isAbsoluteWebUrl()) {
            emptyList()
        } else {
            rows.mapIndexedNotNull { index, row ->
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

        if (!canLogin && actions.isEmpty()) {
            val message = if (loginJs.isAbsoluteWebUrl()) {
                "此书源使用网页登录，不是账号密码表单。"
            } else {
                "此书源没有可用的账号密码登录表单。"
            }
            return buildJsonObject {
                put("status", "unavailable")
                put("message", message)
            }.toString()
        }
        if (rows.mapNotNull { row ->
                row.takeIf { it.type in supportedLoginFieldTypes }?.name
            }.let { it.size != it.toSet().size }
        ) {
            return buildJsonObject {
                put("status", "unavailable")
                put("message", "此书源登录表单包含重复字段名称。")
            }.toString()
        }
        return buildJsonObject {
            put("mode", "form")
            put("canLogin", canLogin)
            put("fields", JsonArray(fields))
            put("actions", JsonArray(actions))
        }.toString()
    }

    /** The URL and existing cookie stay inside the Rust/native login handoff. */
    private fun encodeSourceLoginWebInfo(source: BookSource): String {
        val loginUrl = source.getLoginJs()?.trim().orEmpty()
        check(loginUrl.isAbsoluteWebUrl()) { "This source does not use web login" }
        val cookies = CookieStoreProviders.get()?.getCookie(loginUrl).orEmpty()
        return buildJsonObject {
            put("url", loginUrl)
            put("cookieHeader", cookies)
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
            put("status", "executed")
            put("importedCount", importedCount)
            put("message", if (importedCount == 0) "网页登录已结束，未发现可保存的 Cookie。" else "网页登录 Cookie 已保存。")
        }.toString()
    }

    private suspend fun loginSource(source: BookSource, input: Map<String, String>?): String {
        val loginJs = source.getLoginJs()?.trim().orEmpty()
        if (loginJs.isEmpty() || loginJs.isAbsoluteWebUrl()) {
            return loginResult(
                status = "failed",
                code = "login_unavailable",
                message = "此书源没有可执行的表单登录规则。",
            )
        }
        val rows = source.loginUi().orEmpty()
        val credentials = validateLoginInputs(source, rows, input)
            ?: return loginResult(
                status = "failed",
                code = "invalid_credentials",
                message = "登录输入与此书源表单不匹配。",
            )
        if (rows.any { it.type in supportedLoginFieldTypes }) {
            if (SourceCacheProviders.impl == null) {
                return loginResult(
                    status = "failed",
                    code = "storage_unavailable",
                    message = "登录信息无法保存，请稍后重试。",
                )
            }
            val info = buildJsonObject {
                credentials.forEach { (name, value) -> put(name, value) }
            }.toString()
            if (!source.putLoginInfo(info)) {
                return loginResult(
                    status = "failed",
                    code = "storage_failed",
                    message = "登录信息无法保存，请稍后重试。",
                )
            }
        }
        return try {
            source.login()
            currentCoroutineContext().ensureActive()
            loginResult(
                status = "executed",
                code = "login_executed",
                message = "登录操作已执行。此书源没有提供独立的认证验证。",
            )
        } catch (error: Throwable) {
            currentCoroutineContext().ensureActive()
            loginResult(
                status = "failed",
                code = "login_failed",
                message = "登录请求执行失败。请检查凭据、网络或书源登录规则后重试。",
            )
        }
    }

    private suspend fun runSourceLoginAction(
        source: BookSource,
        actionId: Int?,
        input: Map<String, String>?,
    ): String {
        val rows = source.loginUi().orEmpty()
        val action = actionId?.let(rows::getOrNull)?.takeIf {
            it.type == RowUi.Type.button && !it.action.isNullOrBlank()
                && !it.action.orEmpty().isAbsoluteWebUrl()
        }
        if (action == null) {
            return loginResult(
                status = "failed",
                code = "login_action_unavailable",
                message = "此书源登录操作不可用。",
            )
        }
        val credentials = validateLoginInputs(source, rows, input)
            ?: return loginResult(
                status = "failed",
                code = "invalid_credentials",
                message = "登录输入与此书源表单不匹配。",
            )
        val loginJs = source.getLoginJs().orEmpty()
        if (loginJs.trim().isAbsoluteWebUrl()) {
            return loginResult(
                status = "failed",
                code = "login_action_unavailable",
                message = "This source login action is unavailable.",
            )
        }
        return try {
            source.evalJS("$loginJs\n${action.action}") {
                put("result", { HashMap(credentials) })
            }
            currentCoroutineContext().ensureActive()
            loginResult("executed", "login_action_executed", "登录操作已执行。")
        } catch (error: Throwable) {
            currentCoroutineContext().ensureActive()
            loginResult(
                status = "failed",
                code = "login_action_failed",
                message = "登录操作执行失败。请检查网络或书源登录规则后重试。",
            )
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

    private fun loginResult(status: String, code: String, message: String): String =
        buildJsonObject {
            put("status", status)
            put("code", code)
            put("message", message)
            put("authenticated", JsonNull)
        }.toString()

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
