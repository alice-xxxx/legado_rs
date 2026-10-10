package io.legado.sourceengine.android

import android.app.Activity
import android.app.Dialog
import android.content.Context
import android.net.Uri
import android.util.Log
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.Toast
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.SslErrorHandler
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebStorage
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.webkit.ProfileStore
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import io.legado.sourceengine.bridge.HostBrowserRequest
import io.legado.sourceengine.bridge.HostBrowserResponse
import io.legado.sourceengine.bridge.SourceBrowserSnapshot
import io.legado.sourceengine.bridge.SourceEngineHostWire
import java.io.ByteArrayInputStream
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** One private, offline WebView per Rust-fetched source snapshot. */
internal suspend fun renderSourceBrowserSnapshot(
    context: Context,
    request: HostBrowserRequest,
): HostBrowserResponse {
    SourceEngineHostWire.validateBrowserRequest(request)
    val renderer = AndroidSourceBrowserSnapshot(context.applicationContext, request)
    return try {
        withTimeout(30_000L) { renderer.run() }
    } finally {
        withContext(NonCancellable + Dispatchers.Main.immediate) { renderer.destroy() }
    }
}

private class AndroidSourceBrowserSnapshot(
    private val context: Context,
    private val request: HostBrowserRequest,
) {
    private var webView: WebView? = null
    private var navigationAttempted = false

    suspend fun run(): HostBrowserResponse = withContext(Dispatchers.Main.immediate) {
        val view = WebView(context)
        webView = view
        view.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = false
            blockNetworkLoads = true
            allowFileAccess = false
            allowContentAccess = false
            mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
        }
        awaitBlankDocument(view)
        val prepared = evaluate(view, SourceBrowserSnapshot.prepareDocument(request))
        SourceEngineHostWire.verifyBrowserDocumentPrepared(prepared, request)
        if (request.delayTimeMs > 0) delay(request.delayTimeMs)
        val raw = evaluate(view, SourceBrowserSnapshot.evaluateRule(request))
        check(!navigationAttempted) {
            "Unsupported browser capability used: page navigation"
        }
        SourceEngineHostWire.decodeBrowserResponse(raw, request)
    }

    private suspend fun awaitBlankDocument(view: WebView) = suspendCancellableCoroutine<Unit> { block ->
        view.webViewClient = object : WebViewClient() {
            private var completed = false

            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                if (request.url.toString() == "about:blank") return false
                navigationAttempted = true
                return true
            }

            @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
            override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean {
                if (url == "about:blank") return false
                navigationAttempted = true
                return true
            }

            override fun onPageFinished(view: WebView, url: String) {
                if (!completed && url == "about:blank" && block.isActive) {
                    completed = true
                    block.resume(Unit)
                }
            }

            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                blockNetworkRequest(request)
        }
        block.invokeOnCancellation { view.post { runCatching { view.stopLoading() } } }
        view.loadUrl("about:blank")
    }

    private suspend fun evaluate(view: WebView, script: String): String = suspendCancellableCoroutine { block ->
        try {
            view.evaluateJavascript(script) { result ->
                if (block.isActive) {
                    if (result == null || result == "null") {
                        block.resumeWithException(IllegalStateException("Private browser returned no evaluation result"))
                    } else {
                        block.resume(result)
                    }
                }
            }
        } catch (error: Throwable) {
            if (block.isActive) block.resumeWithException(error)
        }
    }

    fun destroy() {
        webView?.let { view ->
            runCatching { view.stopLoading() }
            runCatching { view.loadUrl("about:blank") }
            runCatching { view.removeAllViews() }
            runCatching { view.destroy() }
        }
        webView = null
    }
}

private fun blockNetworkRequest(request: WebResourceRequest): WebResourceResponse? {
    val scheme = Uri.parse(request.url.toString()).scheme?.lowercase()
    if (scheme in setOf("about", "data", "blob")) return null
    return WebResourceResponse(
        "text/plain",
        "UTF-8",
        403,
        "Blocked by the offline source browser snapshot",
        emptyMap(),
        ByteArrayInputStream(ByteArray(0)),
    )
}

/** Real Android login pages use a disposable WebView profile, never the application's default jar. */
object AndroidSourceWebLogin {
    private const val PROFILE_NAME_PREFIX = "legado-source-login-"
    private const val LOG_TAG = "LegadoSourceLogin"
    private const val SESSION_TTL_MS = 10 * 60 * 1000L
    private const val MAX_COOKIE_HEADER_CHARS = 256 * 1024

    private data class CapturedLogin(
        val loginUrl: String,
        val sourceRevision: Long,
        val restoreEpoch: Long,
        val cookieHeader: String,
    )

    private class LoginSession(
        val sourceId: String,
        val sessionId: String,
        val profileName: String,
        val createdAtMs: Long,
        val loginUrl: String,
        val sourceRevision: Long,
        val restoreEpoch: Long,
        var cookieManager: CookieManager?,
        var webStorage: WebStorage?,
        var dialog: Dialog? = null,
        var container: LinearLayout? = null,
        var webView: WebView? = null,
        var completed: CapturedLogin? = null,
        var captureReady: Boolean = false,
        var captureUnavailable: Boolean = false,
        var finishing: Boolean = false,
        val operationMutex: Mutex = Mutex(),
        val cleanupMutex: Mutex = Mutex(),
    )

    private val sessions = HashMap<String, LoginSession>()
    private val uiScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    /** Starts the native browser and returns as soon as its isolated profile is ready. */
    @JvmStatic
    fun start(
        context: Context,
        sourceId: String,
        sessionId: String,
        sourceRevision: Long,
        restoreEpoch: Long,
        loginUrl: String,
        cookieHeader: String,
    ): Boolean = runBlocking {
        withContext(Dispatchers.Main.immediate) {
            startOnMain(context, sourceId, sessionId, sourceRevision, restoreEpoch, loginUrl, cookieHeader)
        }
    }

    /** Returns cookies captured from the isolated profile, then destroys that profile. */
    @JvmStatic
    fun complete(context: Context, sourceId: String, sessionId: String): String = runBlocking {
        withContext(Dispatchers.Main.immediate) {
            val session = requireSession(sourceId, sessionId)
            val capture = captureAndDisposeOnMain(session, keepResult = false)
            buildJsonObject {
                put("loginUrl", capture.loginUrl)
                put("sourceRevision", capture.sourceRevision)
                put("restoreEpoch", capture.restoreEpoch)
                put("cookieHeader", capture.cookieHeader)
            }.toString()
        }
    }

    /** Discards the temporary profile and any captured cookies without importing them. */
    @JvmStatic
    fun cancel(context: Context, sourceId: String, sessionId: String): Boolean = runBlocking {
        withContext(Dispatchers.Main.immediate) {
            validateSessionIdentity(sourceId, sessionId)
            val session = sessions[sessionId] ?: return@withContext true
            if (session.sourceId != sourceId) error("Private source login session did not match this source")
            cancelSessionOnMain(session)
            true
        }
    }

    private suspend fun startOnMain(
        context: Context,
        sourceId: String,
        sessionId: String,
        sourceRevision: Long,
        restoreEpoch: Long,
        loginUrl: String,
        cookieHeader: String,
    ): Boolean {
        validateSessionInput(sourceId, sessionId, sourceRevision, restoreEpoch, loginUrl, cookieHeader)
        val activity = context as? Activity ?: error("Android source login requires an active Activity")
        check(!activity.isFinishing && !activity.isDestroyed) {
            "Android source login cannot open while the application Activity is closing"
        }
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
            error("This Android System WebView does not support isolated browser profiles. Update Android System WebView to use web login; existing browser cookies will not be shared.")
        }

        val oldSessions = sessions.values.toList()
        oldSessions.forEach { old ->
            cancelSessionOnMain(old)
        }

        check(sessionId !in sessions) { "Private source login session already exists" }
        val profileStore = ProfileStore.getInstance()
        cleanInactiveProfilesOnMain(profileStore)
        val profileName = "$PROFILE_NAME_PREFIX${UUID.randomUUID().toString().replace("-", "")}" 
        val profile = try {
            profileStore.getOrCreateProfile(profileName)
        } catch (error: Throwable) {
            deleteProfileWithRetry(profileStore, profileName)
            throw error
        }
        val (cookieManager, webStorage) = try {
            profile.cookieManager to profile.webStorage
        } catch (error: Throwable) {
            deleteProfileWithRetry(profileStore, profileName)
            throw error
        }
        val session = LoginSession(
            sourceId = sourceId,
            sessionId = sessionId,
            profileName = profileName,
            createdAtMs = System.currentTimeMillis(),
            loginUrl = loginUrl,
            sourceRevision = sourceRevision,
            restoreEpoch = restoreEpoch,
            cookieManager = cookieManager,
            webStorage = webStorage,
        )
        return try {
            cookieManager.setAcceptCookie(true)
            seedCookies(cookieManager, loginUrl, cookieHeader)

            val webView = WebView(activity)
            session.webView = webView
            WebViewCompat.setProfile(webView, profileName)
            webView.settings.apply {
                javaScriptEnabled = true
                domStorageEnabled = true
                allowFileAccess = false
                allowContentAccess = false
                mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
                javaScriptCanOpenWindowsAutomatically = false
                setSupportMultipleWindows(false)
            }
            cookieManager.setAcceptThirdPartyCookies(webView, true)
            webView.webChromeClient = WebChromeClient()
            webView.webViewClient = object : WebViewClient() {
                override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
                    val uri = request.url
                    return !((uri.scheme.equals("http", true) || uri.scheme.equals("https", true))
                        && !uri.host.isNullOrBlank()
                        && uri.userInfo.isNullOrEmpty())
                }

                override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: android.net.http.SslError) {
                    handler.cancel()
                }
            }

            val root = LinearLayout(activity).apply {
                orientation = LinearLayout.VERTICAL
            }
            session.container = root
            root.addView(
                webView,
                LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f),
            )
            val actions = LinearLayout(activity).apply {
                gravity = Gravity.END
                orientation = LinearLayout.HORIZONTAL
                setPadding(12, 8, 12, 8)
            }
            val cancelButton = Button(activity).apply { text = "取消登录" }
            val completeButton = Button(activity).apply { text = "完成登录" }
            actions.addView(cancelButton)
            actions.addView(completeButton)
            root.addView(actions, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))

            val dialog = Dialog(activity).apply {
                setTitle("书源网页登录")
                setContentView(root)
                setCancelable(true)
                setCanceledOnTouchOutside(false)
            }
            session.dialog = dialog
            sessions[sessionId] = session
            cancelButton.setOnClickListener {
                uiScope.launch {
                    runCatching { cancelSessionOnMain(session) }
                        .onSuccess {
                            Toast.makeText(activity, "网页登录已取消，未保存 Cookie。", Toast.LENGTH_SHORT).show()
                        }
                        .onFailure {
                            Toast.makeText(activity, "临时登录数据未能完全清理，请重试。", Toast.LENGTH_LONG).show()
                        }
                }
            }
            completeButton.setOnClickListener {
                uiScope.launch {
                    runCatching { captureAndDisposeOnMain(session, keepResult = true) }
                        .onSuccess {
                            Toast.makeText(activity, "登录页面已关闭，请返回应用保存 Cookie。", Toast.LENGTH_SHORT).show()
                        }
                        .onFailure {
                            session.completed = null
                            session.captureUnavailable = true
                            Toast.makeText(activity, "无法安全完成网页登录，请重试。", Toast.LENGTH_LONG).show()
                        }
                }
            }
            dialog.setOnCancelListener {
                uiScope.launch {
                    runCatching { cancelSessionOnMain(session) }
                }
            }
            dialog.setOnDismissListener {
                if (sessions[sessionId] === session && session.completed == null && !session.finishing) {
                    uiScope.launch {
                        runCatching { cancelSessionOnMain(session) }
                    }
                }
            }

            dialog.show()
            dialog.window?.setLayout(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
            webView.loadUrl(loginUrl)
            uiScope.launch {
                delay(SESSION_TTL_MS)
                if (sessions[sessionId] === session) {
                    repeat(3) { attempt ->
                        val cleaned = runCatching { cancelSessionOnMain(session) }.isSuccess
                        if (cleaned) {
                            return@launch
                        }
                        if (attempt < 2) delay(500L)
                    }
                    Log.w(LOG_TAG, "Private profile cleanup remains pending after session expiry")
                }
            }
            true
        } catch (error: Throwable) {
            sessions.remove(sessionId)
            runCatching { destroyProfileOnMain(session) }
            throw error
        }
    }

    private suspend fun captureAndDisposeOnMain(
        session: LoginSession,
        keepResult: Boolean,
    ): CapturedLogin {
        session.operationMutex.lock()
        try {
            check(sessions[session.sessionId] === session) { "Private source login session has ended" }
            if (session.captureReady) {
                val completed = session.completed ?: error("Private source login capture is missing")
                if (!keepResult) sessions.remove(session.sessionId)
                return completed
            }
            check(!session.captureUnavailable) { "Private source login cookies are unavailable after cleanup failed" }
            if (session.createdAtMs.elapsedSince() >= SESSION_TTL_MS) {
                session.finishing = true
                destroyProfileOnMain(session)
                sessions.remove(session.sessionId)
                error("Private source login session expired")
            }
            val cookieHeader = session.cookieManager?.getCookie(session.loginUrl).orEmpty()
            check(cookieHeader.length <= MAX_COOKIE_HEADER_CHARS
                && cookieHeader.none { it == '\r' || it == '\n' }) {
                "Private browser cookies exceed the supported size"
            }
            val capture = CapturedLogin(session.loginUrl, session.sourceRevision, session.restoreEpoch, cookieHeader)
            session.finishing = true
            try {
                destroyProfileOnMain(session)
            } catch (error: Throwable) {
                session.captureUnavailable = true
                throw error
            }
            session.completed = capture
            session.captureReady = true
            if (!keepResult) sessions.remove(session.sessionId)
            return capture
        } finally {
            session.operationMutex.unlock()
        }
    }

    private suspend fun cancelSessionOnMain(session: LoginSession) {
        session.operationMutex.lock()
        try {
            if (sessions[session.sessionId] !== session) return
            session.completed = null
            session.captureReady = false
            session.captureUnavailable = true
            session.finishing = true
            destroyProfileOnMain(session)
            sessions.remove(session.sessionId)
        } finally {
            session.operationMutex.unlock()
        }
    }

    private suspend fun destroyProfileOnMain(session: LoginSession) {
        session.cleanupMutex.lock()
        try {
            val failures = mutableListOf<Throwable>()
            val webView = session.webView
            if (webView != null) {
                try {
                    session.container?.removeView(webView)
                    session.container = null
                    runCatching { webView.stopLoading() }
                    runCatching { webView.loadUrl("about:blank") }
                    runCatching { webView.removeAllViews() }
                    webView.destroy()
                    session.webView = null
                } catch (error: Throwable) {
                    failures += error
                }
            }
            val dialog = session.dialog
            session.dialog = null
            if (dialog?.isShowing == true) runCatching { dialog.dismiss() }

            session.cookieManager?.let { cookieManager ->
                try {
                    suspendCancellableCoroutine<Unit> { continuation ->
                        try {
                            cookieManager.removeAllCookies {
                                if (continuation.isActive) continuation.resume(Unit)
                            }
                        } catch (error: Throwable) {
                            if (continuation.isActive) continuation.resumeWithException(error)
                        }
                    }
                    cookieManager.flush()
                    session.cookieManager = null
                } catch (error: Throwable) {
                    failures += error
                }
            }
            session.webStorage?.let { webStorage ->
                try {
                    webStorage.deleteAllData()
                    session.webStorage = null
                } catch (error: Throwable) {
                    failures += error
                }
            }

            if (failures.isNotEmpty()) {
                val failure = IllegalStateException("Could not clear isolated source login data")
                failures.forEach(failure::addSuppressed)
                throw failure
            }

            deleteProfileWithRetry(ProfileStore.getInstance(), session.profileName)
        } finally {
            session.cleanupMutex.unlock()
        }
    }

    private suspend fun cleanInactiveProfilesOnMain(profileStore: ProfileStore) {
        val activeNames = sessions.values
            .filter { it.webView != null || it.cookieManager != null || it.webStorage != null }
            .mapTo(HashSet()) { it.profileName }
        val staleNames = profileStore.getAllProfileNames()
            .filter { it.startsWith(PROFILE_NAME_PREFIX) && it !in activeNames }
        for (profileName in staleNames) {
            val profile = profileStore.getProfile(profileName) ?: continue
            val cookieManager = profile.cookieManager
            suspendCancellableCoroutine<Unit> { continuation ->
                try {
                    cookieManager.removeAllCookies {
                        if (continuation.isActive) continuation.resume(Unit)
                    }
                } catch (error: Throwable) {
                    if (continuation.isActive) continuation.resumeWithException(error)
                }
            }
            cookieManager.flush()
            profile.webStorage.deleteAllData()
            deleteProfileWithRetry(profileStore, profileName)
        }
    }

    private suspend fun deleteProfileWithRetry(profileStore: ProfileStore, profileName: String) {
        var lastFailure: Throwable? = null
        repeat(3) { attempt ->
            try {
                profileStore.deleteProfile(profileName)
                return
            } catch (error: Throwable) {
                lastFailure = error
                if (attempt < 2) delay(200L * (attempt + 1))
            }
        }
        Log.w(LOG_TAG, "Could not reclaim empty private source login profile $profileName", lastFailure)
    }

    private fun requireSession(sourceId: String, sessionId: String): LoginSession {
        validateSessionIdentity(sourceId, sessionId)
        val session = sessions[sessionId]
            ?: error("Private source login session has ended")
        if (session.sourceId != sourceId) error("Private source login session did not match this source")
        return session
    }

    private fun validateSessionInput(
        sourceId: String,
        sessionId: String,
        sourceRevision: Long,
        restoreEpoch: Long,
        loginUrl: String,
        cookieHeader: String,
    ) {
        validateSessionIdentity(sourceId, sessionId)
        require(sourceRevision >= 0) { "Source login revision is invalid" }
        require(restoreEpoch >= 0) { "Source login restore epoch is invalid" }
        require(loginUrl.length <= 8192) { "Source login URL exceeds the 8 KiB limit" }
        val uri = Uri.parse(loginUrl)
        require((uri.scheme.equals("http", true) || uri.scheme.equals("https", true))
            && !uri.host.isNullOrBlank()
            && uri.userInfo.isNullOrEmpty()) {
            "Source login URL must be an HTTP(S) URL without credentials"
        }
        require(cookieHeader.length <= MAX_COOKIE_HEADER_CHARS
            && cookieHeader.none { it == '\r' || it == '\n' }) {
            "Stored source cookies exceed the private browser limit"
        }
    }

    private fun validateSessionIdentity(sourceId: String, sessionId: String) {
        require(sourceId.isNotBlank() && sourceId.length <= 256) { "Source login request has an invalid source ID" }
        require(sessionId.matches(Regex("[0-9a-f]{32}"))) { "Private source login session ID is invalid" }
    }

    private suspend fun seedCookies(cookieManager: CookieManager, loginUrl: String, cookieHeader: String) {
        val secure = Uri.parse(loginUrl).scheme.equals("https", true)
        for (rawPair in cookieHeader.split(';')) {
            val (rawName, rawValue) = rawPair.trim().splitOnce('=') ?: continue
            val name = rawName.trim()
            val value = rawValue.trim()
            if (name.isEmpty() || name.any { it.isISOControl() || it == ';' }) continue
            val cookie = buildString {
                append(name).append('=').append(value).append("; Path=/; HttpOnly")
                if (secure) append("; Secure")
            }
            suspendCancellableCoroutine<Unit> { continuation ->
                try {
                    cookieManager.setCookie(loginUrl, cookie) { accepted ->
                        if (continuation.isActive) {
                            if (accepted) continuation.resume(Unit)
                            else continuation.resumeWithException(IllegalStateException("Could not seed private source cookies"))
                        }
                    }
                } catch (error: Throwable) {
                    if (continuation.isActive) continuation.resumeWithException(error)
                }
            }
        }
        cookieManager.flush()
    }

    private fun Long.elapsedSince(): Long = (System.currentTimeMillis() - this).coerceAtLeast(0)

    private fun String.splitOnce(delimiter: Char): Pair<String, String>? {
        val index = indexOf(delimiter)
        if (index < 0) return null
        return substring(0, index) to substring(index + 1)
    }
}
