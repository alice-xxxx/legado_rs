@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.sourceengine.bridge

import io.legado.app.constant.AppConst
import kotlinx.cinterop.ObjCSignatureOverride
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import platform.CoreGraphics.CGRectMake
import platform.Foundation.NSError
import platform.Foundation.NSURL
import platform.WebKit.WKNavigation
import platform.WebKit.WKNavigationAction
import platform.WebKit.WKNavigationActionPolicy
import platform.WebKit.WKNavigationDelegateProtocol
import platform.WebKit.WKWebsiteDataStore
import platform.WebKit.WKWebView
import platform.WebKit.WKWebViewConfiguration
import platform.darwin.NSObject
import platform.darwin.dispatch_async
import platform.darwin.dispatch_get_main_queue
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** A one-shot, non-persistent WKWebView that only renders Rust-fetched HTML snapshots. */
internal suspend fun renderIosSourceBrowserSnapshot(
    request: HostBrowserRequest,
): HostBrowserResponse {
    SourceEngineHostWire.validateBrowserRequest(request)
    val renderer = IosSourceBrowserSnapshot(request)
    return try {
        withTimeout(AppConst.timeLimit.coerceAtLeast(30_000L)) { renderer.run() }
    } finally {
        withContext(NonCancellable + Dispatchers.Main) { renderer.destroy() }
    }
}

private class IosSourceBrowserSnapshot(
    private val request: HostBrowserRequest,
) {
    private var webView: WKWebView? = null
    private var delegate: SnapshotNavigationDelegate? = null
    private var navigationAttempted = false
    private var navigationFailure: Throwable? = null
    private var initialPageLoaded = false

    suspend fun run(): HostBrowserResponse {
        val view = withContext(Dispatchers.Main) { createWebView() }
        withContext(Dispatchers.Main) {
            awaitBlankPage(view)
            val prepared = evaluate(view, SourceBrowserSnapshot.prepareDocument(request))
            SourceEngineHostWire.verifyBrowserDocumentPrepared(prepared, request)
        }
        if (request.delayTimeMs > 0) delay(request.delayTimeMs)
        val raw = withContext(Dispatchers.Main) {
            evaluate(view, SourceBrowserSnapshot.evaluateRule(request))
        }
        navigationFailure?.let { throw it }
        check(!navigationAttempted) { "Unsupported browser capability used: page navigation" }
        return SourceEngineHostWire.decodeBrowserResponse(raw, request)
    }

    private fun createWebView(): WKWebView {
        val configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = WKWebsiteDataStore.nonPersistentDataStore()
        val view = WKWebView(CGRectMake(0.0, 0.0, 1.0, 1.0), configuration)
        val navDelegate = SnapshotNavigationDelegate(
            onFinish = { initialPageLoaded = true },
            onFailure = { navigationFailure = IllegalStateException(it) },
            onBlockedNavigation = { navigationAttempted = true },
        )
        delegate = navDelegate
        view.navigationDelegate = navDelegate
        webView = view
        return view
    }

    private suspend fun awaitBlankPage(view: WKWebView) = suspendCancellableCoroutine<Unit> { block ->
        if (initialPageLoaded) {
            block.resume(Unit)
            return@suspendCancellableCoroutine
        }
        block.invokeOnCancellation {
            dispatch_async(dispatch_get_main_queue()) { view.stopLoading() }
        }
        val blank = "<!doctype html><html><head></head><body></body></html>"
        val listener = delegate
        if (listener != null) {
            listener.onInitialLoad = {
                if (block.isActive) block.resume(Unit)
            }
            listener.onInitialFailure = { error ->
                if (block.isActive) block.resumeWithException(error)
            }
        }
        view.loadHTMLString(blank, NSURL.URLWithString("about:blank"))
    }

    private suspend fun evaluate(view: WKWebView, script: String): String = suspendCancellableCoroutine { block ->
        view.evaluateJavaScript(script) { result, error ->
            if (!block.isActive) return@evaluateJavaScript
            if (error != null) {
                block.resumeWithException(IllegalStateException(error.localizedDescription))
            } else if (result == null) {
                block.resumeWithException(IllegalStateException("Private browser returned no evaluation result"))
            } else {
                block.resume(result.toString())
            }
        }
    }

    fun destroy() {
        webView?.let { view ->
            view.stopLoading()
            view.navigationDelegate = null
        }
        webView = null
        delegate = null
    }
}

private class SnapshotNavigationDelegate(
    private val onFinish: () -> Unit,
    private val onFailure: (String) -> Unit,
    private val onBlockedNavigation: () -> Unit,
) : NSObject(), WKNavigationDelegateProtocol {
    var onInitialLoad: (() -> Unit)? = null
    var onInitialFailure: ((Throwable) -> Unit)? = null

    @ObjCSignatureOverride
    override fun webView(webView: WKWebView, didFinishNavigation: WKNavigation?) {
        onFinish()
        onInitialLoad?.also { onInitialLoad = null }?.invoke()
    }

    @ObjCSignatureOverride
    override fun webView(
        webView: WKWebView,
        didFailProvisionalNavigation: WKNavigation?,
        withError: NSError,
    ) {
        onFailure(withError.localizedDescription)
        onInitialFailure?.also { onInitialFailure = null }?.invoke(IllegalStateException(withError.localizedDescription))
    }

    @ObjCSignatureOverride
    override fun webView(
        webView: WKWebView,
        decidePolicyForNavigationAction: WKNavigationAction,
        decisionHandler: (WKNavigationActionPolicy) -> Unit,
    ) {
        val target = decidePolicyForNavigationAction.request.URL?.absoluteString
        if (target == "about:blank") {
            decisionHandler(WKNavigationActionPolicy.WKNavigationActionPolicyAllow)
        } else {
            onBlockedNavigation()
            decisionHandler(WKNavigationActionPolicy.WKNavigationActionPolicyCancel)
        }
    }
}
