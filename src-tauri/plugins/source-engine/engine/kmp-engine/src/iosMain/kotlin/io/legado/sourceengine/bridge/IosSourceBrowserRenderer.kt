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
import platform.Foundation.NSMutableURLRequest
import platform.Foundation.NSSelectorFromString
import platform.Foundation.setValue
import platform.UIKit.UIApplication
import platform.UIKit.UIViewController
import platform.UIKit.UINavigationController
import platform.UIKit.UIBarButtonItem
import platform.UIKit.UIBarButtonSystemItem.UIBarButtonSystemItemDone
import platform.UIKit.UIModalPresentationFullScreen
import platform.UIKit.navigationController
import platform.UIKit.navigationItem
import kotlinx.cinterop.ObjCAction
import io.legado.app.utils.KS_JSON
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
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
    if (request.mode == HostBrowserMode.INTERACTIVE_PAGE) return openIosSourceScriptBrowser(request)
    val renderer = IosSourceBrowserSnapshot(request)
    return try {
        withTimeout(AppConst.timeLimit.coerceAtLeast(30_000L)) { renderer.run() }
    } finally {
        withContext(NonCancellable + Dispatchers.Main) { renderer.destroy() }
    }
}

private suspend fun openIosSourceScriptBrowser(request: HostBrowserRequest): HostBrowserResponse =
    withTimeout(30 * 60_000L) { withContext(Dispatchers.Main) {
        suspendCancellableCoroutine<HostBrowserResponse> { block ->
            var presenter = UIApplication.sharedApplication.keyWindow?.rootViewController
                ?: error("Source browser requires an active view controller")
            while (presenter.presentedViewController != null) presenter = presenter.presentedViewController!!
            val controller = SourceScriptBrowserController(request) { result ->
                if (block.isActive) result.fold({ block.resume(it) }, { block.resumeWithException(it) })
            }
            val navigation = UINavigationController(rootViewController = controller)
            navigation.modalPresentationStyle = UIModalPresentationFullScreen
            block.invokeOnCancellation {
                dispatch_async(dispatch_get_main_queue()) { controller.cancel() }
            }
            presenter.presentViewController(navigation, animated = true, completion = {
                if (!request.waitForResult && block.isActive) block.resume(HostBrowserResponse(request.capabilityId, request.baseUrl, ""))
            })
        }
    } }

private class SourceScriptBrowserController(
    private val request: HostBrowserRequest,
    private val complete: (Result<HostBrowserResponse>) -> Unit,
) : UIViewController(nibName = null, bundle = null), WKNavigationDelegateProtocol {
    private val browser = WKWebView(frame = CGRectMake(0.0, 0.0, 0.0, 0.0),
        configuration = WKWebViewConfiguration().apply { websiteDataStore = WKWebsiteDataStore.nonPersistentDataStore() })
    private var finishing = false

    override fun viewDidLoad() {
        super.viewDidLoad()
        title = request.title
        view = browser
        browser.navigationDelegate = this
        request.headers.firstOrNull { it.name.equals("User-Agent", true) }?.let { browser.customUserAgent = it.value }
        navigationItem.rightBarButtonItem = UIBarButtonItem(
            barButtonSystemItem = UIBarButtonSystemItemDone, target = this, action = NSSelectorFromString("finishBrowser"))
        val url = NSURL.URLWithString(request.baseUrl) ?: error("Invalid source browser URL")
        val nativeRequest = NSMutableURLRequest(uRL = url)
        request.headers.forEach { nativeRequest.setValue(it.value, forHTTPHeaderField = it.name) }
        browser.loadRequest(nativeRequest)
    }

    @ObjCAction
    fun finishBrowser() {
        if (finishing) return
        finishing = true
        if (!request.waitForResult) { cancel(); return }
        browser.evaluateJavaScript("JSON.stringify({finalUrl:location.href,body:document.documentElement.outerHTML})") { result, error ->
            if (error != null) {
                complete(Result.failure(IllegalStateException(error.localizedDescription))); cancel()
            } else {
                val decoded = runCatching { KS_JSON.parseToJsonElement(result.toString()).jsonObject }
                decoded.fold({ value ->
                    val finalUrl = value.getValue("finalUrl").jsonPrimitive.content
                    val body = value.getValue("body").jsonPrimitive.content
                    browser.configuration.websiteDataStore.httpCookieStore.getAllCookies { cookies ->
                        val host = NSURL.URLWithString(finalUrl)?.host.orEmpty()
                        val cookieHeader = cookies.orEmpty().filterIsInstance<platform.Foundation.NSHTTPCookie>()
                            .filter { host == it.domain.removePrefix(".") || host.endsWith("." + it.domain.removePrefix(".")) }
                            .joinToString("; ") { "${it.name}=${it.value}" }
                        complete(Result.success(HostBrowserResponse(request.capabilityId, finalUrl, body, cookieHeader)))
                        cancel()
                    }
                }, { complete(Result.failure(it)); cancel() })
            }
        }
    }

    fun cancel() {
        browser.stopLoading(); browser.navigationDelegate = null
        navigationController?.dismissViewControllerAnimated(true, completion = null)
    }

    @ObjCSignatureOverride
    override fun webView(webView: WKWebView, decidePolicyForNavigationAction: WKNavigationAction,
        decisionHandler: (WKNavigationActionPolicy) -> Unit) {
        val url = decidePolicyForNavigationAction.request.URL
        val value = url?.absoluteString.orEmpty()
        val allowed = value == request.baseUrl || value == "about:blank" ||
            ((url?.scheme == "http" || url?.scheme == "https") && !url.host.isNullOrBlank() && url.user == null && url.password == null)
        decisionHandler(if (allowed) WKNavigationActionPolicy.WKNavigationActionPolicyAllow else WKNavigationActionPolicy.WKNavigationActionPolicyCancel)
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
