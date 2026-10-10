import Foundation
import LegadoSourceEngine
import SwiftRs
import Tauri
import UIKit
import WebKit

private struct ExecuteArgs: Decodable {
    let requestJson: String
}

/// Tauri's iOS edge enters Kotlin/Native; its HTTP and storage callbacks return through Rust C ABI.
final class SourceEnginePlugin: Plugin {
    private var webLoginSessions: [String: NativeWebLoginSession] = [:]
    private var webLoginExpiry: [String: DispatchWorkItem] = [:]
    private var appInactiveObserver: NSObjectProtocol?

    @objc public func execute(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ExecuteArgs.self)
        let applicationSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!
        let dataDirectory = applicationSupport.appendingPathComponent("source-engine", isDirectory: true).path

        IosSourceEngine.shared.executeJson(
            requestJson: args.requestJson,
            appDataDirectory: dataDirectory
        ) { result, error in
            if let error {
                invoke.reject(error.localizedDescription)
                return
            }
            guard let result else {
                invoke.reject("Kotlin source engine returned no result")
                return
            }
            invoke.resolve(["resultJson": result])
        }
    }

    @objc public func startWebLogin(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(StartWebLoginArgs.self)
        let loginURL = try validateWebLogin(args)
        DispatchQueue.main.async { [weak self] in
            guard let self else {
                invoke.reject("Private login browser is unavailable")
                return
            }
            guard UIApplication.shared.applicationState == .active,
                  let presenter = self.topViewController() else {
                invoke.reject("Cannot present the private login browser while the app is inactive")
                return
            }
            guard self.webLoginSessions.isEmpty else {
                invoke.reject("A private source login browser is already active")
                return
            }

            self.observeAppDeactivation()

            let configuration = WKWebViewConfiguration()
            let dataStore = WKWebsiteDataStore.nonPersistent()
            configuration.websiteDataStore = dataStore
            let webView = WKWebView(frame: .zero, configuration: configuration)
            let controller = NativeWebLoginViewController(webView: webView)
            let navigationController = UINavigationController(rootViewController: controller)
            navigationController.modalPresentationStyle = .pageSheet
            controller.onFinish = { [weak navigationController] in
                navigationController?.dismiss(animated: true)
            }
            controller.onCancel = { [weak self] in
                self?.closeWebLogin(sessionId: args.sessionId, sourceId: args.sourceId)
            }
            webView.navigationDelegate = controller
            webView.uiDelegate = controller

            let session = NativeWebLoginSession(
                sourceId: args.sourceId,
                sessionId: args.sessionId,
                sourceRevision: args.sourceRevision,
                restoreEpoch: args.restoreEpoch,
                sourceLoginUrl: args.loginUrl,
                loginURL: loginURL,
                dataStore: dataStore,
                webView: webView,
                navigationController: navigationController
            )
            self.webLoginSessions[args.sessionId] = session
            presenter.present(navigationController, animated: true) { [weak controller, weak navigationController] in
                navigationController?.presentationController?.delegate = controller
                self.seedAndLoad(session, cookieHeader: args.cookieHeader)
            }
            self.scheduleWebLoginExpiry(session)
            invoke.resolve(["started": true])
        }
    }

    @objc public func completeWebLogin(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(WebLoginSessionArgs.self)
        try validateWebLoginSessionArgs(args)
        DispatchQueue.main.async { [weak self] in
            guard let self,
                  let session = self.webLoginSessions[args.sessionId],
                  session.sourceId == args.sourceId else {
                invoke.reject("Private source login browser session has ended")
                return
            }
            guard Date().timeIntervalSince(session.createdAt) < 10 * 60 else {
                self.closeWebLogin(sessionId: args.sessionId, sourceId: args.sourceId)
                invoke.reject("Private source login browser session expired")
                return
            }

            session.webView.stopLoading()
            session.webView.navigationDelegate = nil
            session.webView.uiDelegate = nil
            session.dataStore.httpCookieStore.getAllCookies { [weak self] cookies in
                DispatchQueue.main.async {
                    guard let self,
                          UIApplication.shared.applicationState == .active,
                          self.webLoginSessions[args.sessionId] === session else {
                        invoke.reject("Private source login browser session has ended")
                        return
                    }
                    guard Date().timeIntervalSince(session.createdAt) < 10 * 60 else {
                        self.closeWebLogin(sessionId: args.sessionId, sourceId: args.sourceId)
                        invoke.reject("Private source login browser session expired")
                        return
                    }
                    self.webLoginExpiry.removeValue(forKey: args.sessionId)?.cancel()
                    self.webLoginSessions.removeValue(forKey: args.sessionId)
                    let header = cookies
                        .filter { self.cookie($0, appliesTo: session.loginURL) }
                        .map { "\($0.name)=\($0.value)" }
                        .joined(separator: "; ")
                    self.dismissWebLogin(session)
                    self.clearWebLoginCookies(session.dataStore)
                    guard header.utf8.count <= 256 * 1024,
                          !header.contains("\r"), !header.contains("\n") else {
                        invoke.reject("Private browser cookies exceed the supported size")
                        return
                    }
                    invoke.resolve([
                        "loginUrl": session.sourceLoginUrl,
                        "sourceRevision": session.sourceRevision,
                        "restoreEpoch": session.restoreEpoch,
                        "cookieHeader": header,
                    ])
                }
            }
        }
    }

    @objc public func cancelWebLogin(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(WebLoginSessionArgs.self)
        try validateWebLoginSessionArgs(args)
        DispatchQueue.main.async { [weak self] in
            let cancelled = self?.closeWebLogin(sessionId: args.sessionId, sourceId: args.sourceId) ?? false
            invoke.resolve(["cancelled": cancelled])
        }
    }

    deinit {
        if let appInactiveObserver {
            NotificationCenter.default.removeObserver(appInactiveObserver)
        }
    }

    private func validateWebLogin(_ args: StartWebLoginArgs) throws -> Foundation.URL {
        try validateWebLoginSessionArgs(WebLoginSessionArgs(sourceId: args.sourceId, sessionId: args.sessionId))
        guard args.sourceId.utf8.count <= 256,
              args.cookieHeader.utf8.count <= 256 * 1024,
              !args.cookieHeader.contains("\r"), !args.cookieHeader.contains("\n"),
              args.loginUrl.utf8.count <= 8192,
              let components = Foundation.URLComponents(string: args.loginUrl),
              let scheme = components.scheme?.lowercased(), scheme == "http" || scheme == "https",
              components.host != nil, components.user == nil, components.password == nil,
              let url = components.url else {
            throw NSError(domain: "SourceEngine", code: 1, userInfo: [NSLocalizedDescriptionKey: "Source login URL or private browser input is invalid"])
        }
        return url
    }

    private func validateWebLoginSessionArgs(_ args: WebLoginSessionArgs) throws {
        let allowed = CharacterSet(charactersIn: "0123456789abcdef")
        guard !args.sourceId.isEmpty, args.sourceId.utf8.count <= 256,
              args.sessionId.utf8.count == 32,
              args.sessionId.unicodeScalars.allSatisfy({ allowed.contains($0) }) else {
            throw NSError(domain: "SourceEngine", code: 2, userInfo: [NSLocalizedDescriptionKey: "Private login browser session identity is invalid"])
        }
    }

    private func observeAppDeactivation() {
        guard appInactiveObserver == nil else { return }
        appInactiveObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.willResignActiveNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            guard let self else { return }
            for sessionId in Array(self.webLoginSessions.keys) {
                self.closeWebLogin(sessionId: sessionId)
            }
        }
    }

    private func topViewController() -> UIViewController? {
        let windows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap { $0.windows }
        guard let root = windows.first(where: \.isKeyWindow)?.rootViewController else { return nil }
        return topViewController(root)
    }

    private func topViewController(_ controller: UIViewController) -> UIViewController {
        if let presented = controller.presentedViewController { return topViewController(presented) }
        if let navigation = controller as? UINavigationController,
           let visible = navigation.visibleViewController { return topViewController(visible) }
        if let tabs = controller as? UITabBarController,
           let selected = tabs.selectedViewController { return topViewController(selected) }
        return controller
    }

    private func seedAndLoad(_ session: NativeWebLoginSession, cookieHeader: String) {
        let host = session.loginURL.host ?? ""
        let cookies = cookieHeader.split(separator: ";").compactMap { part -> HTTPCookie? in
            let pair = part.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let separator = pair.firstIndex(of: "=") else { return nil }
            let name = String(pair[..<separator]).trimmingCharacters(in: .whitespaces)
            let value = String(pair[pair.index(after: separator)...]).trimmingCharacters(in: .whitespaces)
            guard !name.isEmpty,
                  !name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else { return nil }
            let properties: [HTTPCookiePropertyKey: Any] = [
                .domain: host,
                .path: "/",
                .name: name,
                .value: value,
                HTTPCookiePropertyKey("Secure"): session.loginURL.scheme?.lowercased() == "https" ? "TRUE" : "FALSE",
                HTTPCookiePropertyKey("HttpOnly"): "TRUE",
            ]
            return HTTPCookie(properties: properties)
        }
        let store = session.dataStore.httpCookieStore
        let group = DispatchGroup()
        for cookie in cookies {
            group.enter()
            store.setCookie(cookie) { group.leave() }
        }
        group.notify(queue: .main) { [weak self, weak session] in
            guard let self, let session,
                  self.webLoginSessions[session.sessionId] === session else { return }
            session.webView.load(URLRequest(url: session.loginURL))
        }
    }

    @discardableResult
    private func closeWebLogin(sessionId: String, sourceId: String? = nil) -> Bool {
        guard let session = webLoginSessions[sessionId], sourceId == nil || session.sourceId == sourceId else { return false }
        webLoginSessions.removeValue(forKey: sessionId)
        webLoginExpiry.removeValue(forKey: sessionId)?.cancel()
        session.webView.stopLoading()
        session.webView.navigationDelegate = nil
        session.webView.uiDelegate = nil
        dismissWebLogin(session)
        clearWebLoginCookies(session.dataStore)
        return true
    }

    private func scheduleWebLoginExpiry(_ session: NativeWebLoginSession) {
        let expiry = DispatchWorkItem { [weak self, weak session] in
            guard let self, let session,
                  self.webLoginSessions[session.sessionId] === session,
                  Date().timeIntervalSince(session.createdAt) >= 10 * 60 else { return }
            self.closeWebLogin(sessionId: session.sessionId)
        }
        webLoginExpiry[session.sessionId] = expiry
        DispatchQueue.main.asyncAfter(deadline: .now() + 10 * 60, execute: expiry)
    }

    private func dismissWebLogin(_ session: NativeWebLoginSession) {
        session.navigationController?.dismiss(animated: true)
        session.navigationController = nil
    }

    private func clearWebLoginCookies(_ dataStore: WKWebsiteDataStore) {
        let store = dataStore.httpCookieStore
        store.getAllCookies { cookies in
            for cookie in cookies { store.delete(cookie) {} }
        }
    }

    private func cookie(_ cookie: HTTPCookie, appliesTo url: Foundation.URL) -> Bool {
        let host = (url.host ?? "").lowercased()
        let domain = cookie.domain.trimmingCharacters(in: CharacterSet(charactersIn: ".")).lowercased()
        guard host == domain || host.hasSuffix("." + domain) else { return false }
        let requestPath = url.path.isEmpty ? "/" : url.path
        let cookiePath = cookie.path.isEmpty ? "/" : cookie.path
        let pathMatches = requestPath == cookiePath || requestPath.hasPrefix(cookiePath.hasSuffix("/") ? cookiePath : cookiePath + "/")
        guard pathMatches else { return false }
        if cookie.isSecure && url.scheme?.lowercased() != "https" { return false }
        if let expires = cookie.expiresDate, expires <= Date() { return false }
        return true
    }
}

private struct StartWebLoginArgs: Decodable {
    let sourceId: String
    let sessionId: String
    let sourceRevision: UInt64
    let restoreEpoch: UInt64
    let loginUrl: String
    let cookieHeader: String
}

private struct WebLoginSessionArgs: Decodable {
    let sourceId: String
    let sessionId: String
}

private final class NativeWebLoginSession {
    let sourceId: String
    let sessionId: String
    let sourceRevision: UInt64
    let restoreEpoch: UInt64
    let sourceLoginUrl: String
    let loginURL: Foundation.URL
    let dataStore: WKWebsiteDataStore
    let webView: WKWebView
    let createdAt = Date()
    weak var navigationController: UINavigationController?

    init(sourceId: String, sessionId: String, sourceRevision: UInt64, restoreEpoch: UInt64,
         sourceLoginUrl: String, loginURL: Foundation.URL,
         dataStore: WKWebsiteDataStore, webView: WKWebView, navigationController: UINavigationController) {
        self.sourceId = sourceId
        self.sessionId = sessionId
        self.sourceRevision = sourceRevision
        self.restoreEpoch = restoreEpoch
        self.sourceLoginUrl = sourceLoginUrl
        self.loginURL = loginURL
        self.dataStore = dataStore
        self.webView = webView
        self.navigationController = navigationController
    }
}

private final class NativeWebLoginViewController: UIViewController, WKNavigationDelegate, WKUIDelegate,
    UIAdaptivePresentationControllerDelegate {
    private let webView: WKWebView
    private var isFinishing = false
    var onFinish: (() -> Void)?
    var onCancel: (() -> Void)?

    init(webView: WKWebView) {
        self.webView = webView
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "书源网页登录"
        view.backgroundColor = .systemBackground
        navigationItem.leftBarButtonItem = UIBarButtonItem(title: "取消", style: .plain, target: self, action: #selector(cancelLogin))
        navigationItem.rightBarButtonItem = UIBarButtonItem(title: "完成", style: .done, target: self, action: #selector(finishLogin))
        webView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(webView)
        NSLayoutConstraint.activate([
            webView.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            webView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            webView.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
        ])
    }

    @objc private func cancelLogin() { onCancel?() }
    @objc private func finishLogin() {
        isFinishing = true
        onFinish?()
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        if !isFinishing { onCancel?() }
    }

    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
                 decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
        guard let url = navigationAction.request.url,
              let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
              url.host != nil,
              URLComponents(url: url, resolvingAgainstBaseURL: false)?.user == nil,
              URLComponents(url: url, resolvingAgainstBaseURL: false)?.password == nil else {
            decisionHandler(.cancel)
            return
        }
        decisionHandler(.allow)
    }

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        guard let url = navigationAction.request.url,
              let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
              url.host != nil else { return nil }
        webView.load(navigationAction.request)
        return nil
    }
}

@_cdecl("init_plugin_source_engine")
func initPlugin() -> Plugin {
    SourceEnginePlugin()
}
