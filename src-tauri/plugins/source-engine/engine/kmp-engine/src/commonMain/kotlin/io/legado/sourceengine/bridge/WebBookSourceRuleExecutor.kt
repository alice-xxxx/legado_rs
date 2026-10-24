package io.legado.sourceengine.bridge

import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BaseSource
import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.rule.RowUi
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
import io.legado.app.utils.NetworkUtils
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

/**
 * 把 Rust 请求映射到共享的 WebBook 编排入口。
 *
 * 解析顺序和字段语义由 WebBook / AnalyzeUrlCore 决定，对外提供独立、类型明确的引擎方法。
 * 宿主跟随本次协程传递，避免并发请求之间覆盖全局 HTTP 回调目标。
 */
@OptIn(ExperimentalEncodingApi::class)
object WebBookSourceRuleExecutor : SourceRuleExecutor {
    override suspend fun search(request: SearchRequest, host: SourceEngineHost): SearchPage = hosted(host) {
        kotlinx.coroutines.withTimeout(10_000L) {
            val page = WebBook.getBookListAwait(request.source, request.keyword, request.page)
            SearchPage(page.books, page.hasNextPage)
        }
    }

    override suspend fun exploreKinds(request: SourceRequest, host: SourceEngineHost): ExploreKinds =
        hosted(host) { ExploreKinds(request.source.exploreKinds()) }

    override suspend fun explore(request: ExploreRequest, host: SourceEngineHost): SearchPage = hosted(host) {
        val page = WebBook.getBookListAwait(request.source, request.url, request.page, isSearch = false)
        SearchPage(page.books, page.hasNextPage)
    }

    override suspend fun detail(request: BookRequest, host: SourceEngineHost): Book =
        hosted(host) { WebBook.getBookInfoAwait(request.source, request.book) }

    override suspend fun chapters(request: BookRequest, host: SourceEngineHost): ChapterList =
        hosted(host) { ChapterList(WebBook.getChapterListAwait(request.source, request.book).getOrThrow()) }

    override suspend fun content(request: ContentRequest, host: SourceEngineHost): ChapterContent = hosted(host) {
        ChapterContent(WebBook.getContentAwait(
            bookSource = request.source, book = request.book, bookChapter = request.chapter,
            nextChapterUrl = request.nextChapterUrl,
        ))
    }

    override suspend fun resolveMedia(request: MediaRequest, host: SourceEngineHost): MediaLocation = hosted(host) {
        val (url, headers) = AnalyzeUrlFactories.create(
            rawUrl = request.url, source = request.source, ruleData = request.book,
            chapter = request.chapter, coroutineContext = currentCoroutineContext(), headerMapF = request.headers,
        ).resolveMedia()
        MediaLocation(url, headers)
    }

    override suspend fun httpTtsAudio(request: TtsRequest, host: SourceEngineHost): AudioData = hosted(host) {
        check(request.text.isNotBlank() && request.text.length <= 20_000 && request.text.none {
            it == '\u0000' || (it.isISOControl() && it !in "\n\r\t")
        }) { "HTTP TTS text must contain 1–20000 characters without invalid control characters" }
        check(request.speechRate in 0..45) { "HTTP TTS speech rate must be between 0 and 45" }
        val response = HttpTtsRequest.audioResponse(
            config = request.config, text = request.text, speechRate = request.speechRate,
            context = currentCoroutineContext(),
        )
        try {
            check(response.header("Content-Length")?.toLongOrNull()?.let { it <= MAX_HTTP_TTS_AUDIO_BYTES } != false) {
                "HTTP TTS audio exceeds the 16 MiB limit"
            }
            val bytes = response.body.bytes()
            check(bytes.isNotEmpty()) { "HTTP TTS returned an empty audio response" }
            check(bytes.size <= MAX_HTTP_TTS_AUDIO_BYTES) { "HTTP TTS audio exceeds the 16 MiB limit" }
            AudioData(Base64.encode(bytes), response.header("Content-Type")?.substringBefore(';')?.trim().orEmpty())
        } finally {
            response.close()
        }
    }

    override suspend fun login(request: LoginRequest, host: SourceEngineHost): LoginState =
        hosted(host) { sourceLogin(request, host) }

    private suspend fun <T> hosted(host: SourceEngineHost, block: suspend () -> T): T = try {
        SourceEngineHostRegistry.withHost(host) {
            withContext(SourceEngineHostContext(host)) { block() }
        }
    } catch (error: Throwable) {
        if (error is Exception) throw error
        throw IllegalStateException(error.message ?: "Source rule failed", error)
    }

    private const val MAX_HTTP_TTS_AUDIO_BYTES = 16 * 1024 * 1024

    private suspend fun sourceLogin(request: LoginRequest, host: SourceEngineHost): LoginState {
        val source = request.source
        val loginUrl = source.getLoginJs()?.trim().orEmpty()
        val web = loginUrl.isAbsoluteWebUrl()
        val result = when (val action = request.action) {
            LoginAction.Open -> LoginState()
            is LoginAction.Submit -> if (web) LoginState() else withSourceLoginUi { submitLogin(source, action) }
            is LoginAction.ImportCookies -> importSourceLoginCookies(source, action.url, action.cookies)
            LoginAction.Clear -> {
                val store = CookieStoreProviders.get() ?: error("Source cookie store is not installed")
                store.removeCookieStrict(source.getKey())
                source.removeLoginHeader()
                source.removeLoginInfo()
                LoginState(message = "已清除登录数据。")
            }
        }
        currentCoroutineContext().ensureActive()
        val sourceUrl = source.getKey()
        val scope = NetworkUtils.getSubDomain(sourceUrl)
        val cache = SourceCacheProviders.impl ?: error("Source cache is not installed")
        val cookies = CookieStoreProviders.get()
        val saved = !cache.get("loginHeader_$sourceUrl").isNullOrBlank()
            || !cache.get("userInfo_$sourceUrl").isNullOrBlank()
            || !cookies?.getCookieNoSession(sourceUrl).isNullOrBlank()
            || !cookies?.getSessionCookie(scope).isNullOrBlank()
            || host.storage.hasCookieJarCookies(scope, sourceUrl)
        return result.copy(
            hasLoginState = saved,
            form = if (request.action == LoginAction.Open || "login" in result.refreshTargets) sourceLoginForm(source) else result.form,
            web = if (web && (request.action == LoginAction.Open || request.action is LoginAction.Submit)) {
                LoginWeb(loginUrl, cookies?.getCookie(loginUrl).orEmpty())
            } else result.web,
        )
    }

    /** 普通登录和表单按钮共享输入校验、错误处理及回填，只在执行脚本处区分。 */
    private suspend fun submitLogin(source: BookSource, call: LoginAction.Submit): LoginState = try {
        val rows = source.loginUi().orEmpty()
        val credentials = requireNotNull(validateLoginInputs(source, rows, call.fields)) {
            "登录输入与此书源表单不匹配。"
        }
        val script = source.getLoginJs().orEmpty()
        require(!script.trim().isAbsoluteWebUrl()) { "此书源使用网页登录。" }
        if (call.button == null) {
            require(script.isNotBlank()) { "此书源没有可执行的登录规则。" }
            if (rows.any { it.type in supportedLoginFieldTypes }) {
                check(SourceCacheProviders.impl != null) { "登录存储不可用。" }
                val info = buildJsonObject { credentials.forEach { (name, value) -> put(name, value) } }
                check(source.putLoginInfo(info.toString())) { "登录信息无法保存。" }
            }
            source.login()
        } else {
            val button = requireNotNull(rows.getOrNull(call.button)?.takeIf {
                it.type == RowUi.Type.button && !it.action.isNullOrBlank() && !it.action.orEmpty().isAbsoluteWebUrl()
            }) { "此书源登录操作不可用。" }
            source.evalJS("$script\n${button.action}") { put("result", HashMap(credentials)) }
        }
        currentCoroutineContext().ensureActive()
        LoginState(
            message = "登录操作已执行。",
            inputUpdates = rows.filter { it.type == RowUi.Type.password }.associate { it.name to "" },
        )
    } catch (error: Throwable) {
        currentCoroutineContext().ensureActive()
        LoginState(error = error.message?.take(4096) ?: "登录操作失败。")
    }

    private fun sourceLoginForm(source: BookSource): LoginForm {
        val loginJs = source.getLoginJs()?.trim().orEmpty()
        if (loginJs.isAbsoluteWebUrl()) return LoginForm(mode = "web")
        val canLogin = loginJs.isNotEmpty()
        val rows = source.loginUi().orEmpty()
        val actions = rows.mapIndexedNotNull { index, row ->
            row.takeIf {
                it.type == RowUi.Type.button && !it.action.isNullOrBlank() && !it.action.orEmpty().isAbsoluteWebUrl()
            }?.let { LoginButton(index, it.name) }
        }
        val fields = rows.mapNotNull { row ->
            val type = when (row.type) {
                RowUi.Type.text -> "text"
                RowUi.Type.password -> "password"
                RowUi.Type.select -> "select"
                RowUi.Type.toggle -> "toggle"
                else -> return@mapNotNull null
            }
            LoginField(row.name, row.name, type, type == "password", if (type == "select") row.chars.orEmpty() else emptyList())
        }
        require(canLogin || actions.isNotEmpty()) { "此书源没有可用的登录表单。" }
        require(fields.map { it.name }.let { it.size == it.toSet().size }) { "此书源登录表单包含重复字段名称。" }
        val actionIds = actions.map { it.id }.toSet()
        val layout = rows.mapIndexedNotNull { index, row ->
            val kind = when {
                row.type in supportedLoginFieldTypes -> "field"
                row.type == RowUi.Type.button && index in actionIds -> "action"
                row.type == RowUi.Type.title && row.name.isNotBlank() -> "title"
                else -> return@mapIndexedNotNull null
            }
            val basis = row.style?.layout_flexBasisPercent ?: 0F
            val legacyCols = when {
                !basis.isFinite() || basis <= 0F -> null
                basis >= 0.75F -> 1
                basis >= 0.42F -> 2
                basis >= 0.29F -> 3
                else -> 4
            }
            LoginLayout(
                kind = kind, name = row.name.takeIf { kind == "field" }, id = index.takeIf { kind == "action" },
                label = row.name.takeIf { kind == "title" },
                cols = (row.style?.cols ?: legacyCols ?: if (kind == "action") 2 else 1).coerceIn(1, 4),
                rows = (row.style?.rows ?: 1).coerceIn(1, 12),
            )
        }
        return LoginForm("form", canLogin, fields, actions, layout)
    }

    /** Import only cookies returned by the native private browser for this source's login URL. */
    private fun importSourceLoginCookies(
        source: BookSource,
        cookieUrl: String?,
        cookieHeader: String,
    ): LoginState {
        val loginUrl = source.getLoginJs()?.trim().orEmpty()
        check(loginUrl.isAbsoluteWebUrl() && loginUrl == cookieUrl) {
            "Source login URL changed while the private browser was open"
        }
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
        return LoginState(message = if (importedCount == 0) "网页登录已结束，未发现可保存的 Cookie。" else "网页登录 Cookie 已保存。")
    }

    /** 注册原版 JS 使用的 UI 能力，浏览器等待及 Cookie 交给各平台宿主。 */
    private suspend fun withSourceLoginUi(block: suspend () -> LoginState): LoginState {
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
            val result = block()
            result.copy(
                messages = messages.takeIf { it.isNotEmpty() }?.toList() ?: result.messages,
                inputUpdates = result.inputUpdates + updates,
                refreshTargets = refreshTargets.takeIf { it.isNotEmpty() }?.toList() ?: result.refreshTargets,
                copyText = copyText ?: result.copyText,
            )
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

}
