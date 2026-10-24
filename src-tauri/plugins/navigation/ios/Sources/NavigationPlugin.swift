import Tauri
import UIKit
import WebKit

private struct GesturesArgs: Decodable {
    let enabled: Bool
    let local: Bool
}

final class NavigationPlugin: Plugin {
    private weak var webview: WKWebView?
    private lazy var localBackGesture: UIScreenEdgePanGestureRecognizer = {
        let gesture = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(handleLocalBack(_:)))
        gesture.edges = .left
        gesture.isEnabled = false
        return gesture
    }()

    override public func load(webview: WKWebView) {
        self.webview = webview
        webview.allowsBackForwardNavigationGestures = false
        webview.addGestureRecognizer(localBackGesture)
        webview.scrollView.panGestureRecognizer.require(toFail: localBackGesture)
    }

    @objc private func handleLocalBack(_ gesture: UIScreenEdgePanGestureRecognizer) {
        guard let webview else { return }
        let distance = gesture.translation(in: webview).x
        let progress = max(0, min(1, distance / max(1, webview.bounds.width)))
        let phase: String
        var commit = false
        switch gesture.state {
        case .began: phase = "start"
        case .changed: phase = "move"
        case .ended:
            phase = "end"
            commit = distance >= max(48, webview.bounds.width * 0.2)
                || distance > 12 && gesture.velocity(in: webview).x > 500
        case .cancelled, .failed: phase = "end"
        default: return
        }
        trigger("backGesture", data: ["phase": phase, "progress": Double(progress), "commit": commit])
    }

    @objc public func setNavigationGestures(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(GesturesArgs.self)
        DispatchQueue.main.async { [weak self] in
            guard let self, let webview = self.webview else {
                invoke.reject("WebView is unavailable")
                return
            }
            webview.allowsBackForwardNavigationGestures = args.enabled && !args.local
            self.localBackGesture.isEnabled = args.enabled && args.local
            invoke.resolve()
        }
    }
}

@_cdecl("init_plugin_navigation")
func initPlugin() -> Plugin {
    NavigationPlugin()
}
