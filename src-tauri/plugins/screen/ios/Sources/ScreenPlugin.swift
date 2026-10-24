import Foundation
import Tauri
import UIKit
import WebKit

private struct ScreenOptions: Decodable {
    let fullscreen: Bool?
    let systemUiHidden: Bool?
    let systemUiStyle: String?
    let orientation: String?
    let keepAwake: Bool?
    let brightness: Double?
}
private struct ApplyArgs: Decodable { let options: ScreenOptions }

final class ScreenPlugin: Plugin {
    private weak var webview: WKWebView?
    private var originalHidden: Bool?
    private var originalInset: UIScrollView.ContentInsetAdjustmentBehavior?
    private var originalStyle: UIStatusBarStyle?
    private var originalOrientation: UIInterfaceOrientationMask?
    private var originalSupportedOrientations: UIInterfaceOrientationMask?
    private var orientationVersion: UInt64 = 0
    private var originalIdleTimer: Bool?
    private var originalBrightness: CGFloat?
    private var brightnessOverride: CGFloat?
    private var observers: [NSObjectProtocol] = []
    private var canChangeBars: Bool {
        (Bundle.main.object(forInfoDictionaryKey: "UIViewControllerBasedStatusBarAppearance") as? Bool) == false
    }

    override public func load(webview: WKWebView) {
        self.webview = webview
        guard observers.isEmpty else { return }
        for name in [UIApplication.willResignActiveNotification, UIApplication.didBecomeActiveNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] note in
                guard let self else { return }
                if note.name == UIApplication.willResignActiveNotification {
                    if let original = self.originalBrightness { self.screen.brightness = original; self.originalBrightness = nil }
                } else if let level = self.brightnessOverride {
                    if self.originalBrightness == nil { self.originalBrightness = self.screen.brightness }
                    self.screen.brightness = level
                }
            })
        }
    }
    deinit { observers.forEach { NotificationCenter.default.removeObserver($0) } }
    private var screen: UIScreen { webview?.window?.screen ?? UIScreen.main }

    private func setHidden(_ hidden: Bool) {
        if originalHidden == nil { originalHidden = UIApplication.shared.isStatusBarHidden }
        if let scroll = webview?.scrollView {
            if originalInset == nil { originalInset = scroll.contentInsetAdjustmentBehavior }
            scroll.contentInsetAdjustmentBehavior = hidden ? .never : originalInset!
        }
        UIApplication.shared.setStatusBarHidden(hidden, with: .none)
    }

    private func setOrientation(_ value: String) -> Bool {
        let selector = NSSelectorFromString("setSupportedInterfaceOrientations:")
        guard let window = webview?.window, let scene = window.windowScene,
              let controller = window.rootViewController, controller.responds(to: selector),
              let implementation = controller.method(for: selector) else { return false }
        let target: UIInterfaceOrientationMask
        switch value {
        case "portrait": target = .portrait.union(.portraitUpsideDown)
        case "landscape": target = .landscape
        case "portrait-primary": target = .portrait
        case "portrait-secondary": target = .portraitUpsideDown
        case "landscape-primary": target = .landscapeLeft
        case "landscape-secondary": target = .landscapeRight
        case "auto": target = .all
        case "restore":
            guard let original = originalOrientation else { return true }
            target = original
        default: return false
        }
        let supported = originalSupportedOrientations ?? controller.supportedInterfaceOrientations
        let requested = target.intersection(supported)
        guard !requested.isEmpty else { return false }
        if originalOrientation == nil {
            originalSupportedOrientations = supported
            switch scene.interfaceOrientation {
            case .landscapeLeft: originalOrientation = .landscapeLeft
            case .landscapeRight: originalOrientation = .landscapeRight
            case .portraitUpsideDown: originalOrientation = .portraitUpsideDown
            default: originalOrientation = .portrait
            }
        }
        // Tauri's window controller already exposes this setter; reuse its policy.
        typealias Setter = @convention(c) (AnyObject, Selector, UInt) -> Void
        unsafeBitCast(implementation, to: Setter.self)(controller, selector, value == "restore" ? supported.rawValue : requested.rawValue)
        orientationVersion &+= 1
        let version = orientationVersion
        if #available(iOS 16.0, *) {
            controller.setNeedsUpdateOfSupportedInterfaceOrientations()
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: requested)) { [weak self] error in
                guard let self, self.orientationVersion == version else { return }
                self.trigger("error", data: ["operation": "orientation", "message": error.localizedDescription])
                NSLog("Screen orientation request failed: %@", error.localizedDescription)
            }
        }
        if value == "restore" { originalOrientation = nil; originalSupportedOrientations = nil }
        return true
    }

    private func setBrightness(_ value: Double) {
        if value == -1 {
            if let original = originalBrightness { screen.brightness = original }
            originalBrightness = nil
            brightnessOverride = nil
        } else {
            brightnessOverride = CGFloat(value)
            if UIApplication.shared.applicationState == .active {
                if originalBrightness == nil { originalBrightness = screen.brightness }
                screen.brightness = CGFloat(value)
            }
        }
    }

    private func onMain(_ invoke: Invoke, _ action: @escaping (ScreenPlugin) -> Void) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.webview != nil else { invoke.reject("WebView is unavailable"); return }
            action(self)
        }
    }

    @objc public func apply(_ invoke: Invoke) throws {
        let options = try invoke.parseArgs(ApplyArgs.self).options
        onMain(invoke) { plugin in
            var applied: [String] = [], unsupported: [String] = []
            for (key, value) in [("fullscreen", options.fullscreen), ("systemUiHidden", options.systemUiHidden)] {
                if let value {
                    if plugin.canChangeBars { plugin.setHidden(value); applied.append(key) }
                    else { unsupported.append(key) }
                }
            }
            if let style = options.systemUiStyle {
                if plugin.canChangeBars {
                    if plugin.originalStyle == nil { plugin.originalStyle = UIApplication.shared.statusBarStyle }
                    UIApplication.shared.setStatusBarStyle(style == "light" ? .lightContent : style == "dark" ? .darkContent : .default, animated: false)
                    applied.append("systemUiStyle")
                } else { unsupported.append("systemUiStyle") }
            }
            if let orientation = options.orientation {
                if plugin.setOrientation(orientation) { applied.append("orientation") }
                else { unsupported.append("orientation") }
            }
            if let enabled = options.keepAwake {
                if plugin.originalIdleTimer == nil { plugin.originalIdleTimer = UIApplication.shared.isIdleTimerDisabled }
                UIApplication.shared.isIdleTimerDisabled = enabled
                applied.append("keepAwake")
            }
            if let value = options.brightness { plugin.setBrightness(value); applied.append("brightness") }
            invoke.resolve(["applied": applied, "unsupported": unsupported])
        }
    }

    @objc public func getInfo(_ invoke: Invoke) {
        onMain(invoke) { plugin in
            let bounds = plugin.webview!.bounds, insets = plugin.webview!.safeAreaInsets
            let scene = plugin.webview?.window?.windowScene
            let hidden = scene?.statusBarManager?.isStatusBarHidden ?? UIApplication.shared.isStatusBarHidden
            let style = scene?.statusBarManager?.statusBarStyle ?? UIApplication.shared.statusBarStyle
            let orientationSupported = plugin.webview?.window?.rootViewController?.responds(to: NSSelectorFromString("setSupportedInterfaceOrientations:")) == true
            let safeArea: JSObject = ["top": Double(insets.top), "right": Double(insets.right), "bottom": Double(insets.bottom), "left": Double(insets.left)]
            let capabilities: JSObject = ["fullscreen": plugin.canChangeBars, "systemUiHidden": plugin.canChangeBars,
                "systemUiStyle": plugin.canChangeBars, "orientation": orientationSupported, "keepAwake": true, "brightness": true]
            invoke.resolve([
                "width": Double(bounds.width), "height": Double(bounds.height), "scale": Double(plugin.screen.scale),
                "fullscreen": hidden, "systemUiHidden": hidden,
                "systemUiStyle": style == .lightContent ? "light" : style == .darkContent ? "dark" : "auto",
                "keepAwake": UIApplication.shared.isIdleTimerDisabled, "brightness": Double(plugin.screen.brightness),
                "orientation": scene?.interfaceOrientation.isLandscape == true ? "landscape" : "portrait",
                "safeArea": safeArea, "capabilities": capabilities
            ])
        }
    }

    @objc public func reset(_ invoke: Invoke) {
        onMain(invoke) { plugin in
            if let original = plugin.originalHidden { UIApplication.shared.setStatusBarHidden(original, with: .none) }
            if let original = plugin.originalInset { plugin.webview?.scrollView.contentInsetAdjustmentBehavior = original }
            if let original = plugin.originalStyle { UIApplication.shared.setStatusBarStyle(original, animated: false) }
            if let original = plugin.originalIdleTimer { UIApplication.shared.isIdleTimerDisabled = original }
            _ = plugin.setOrientation("restore")
            plugin.setBrightness(-1)
            plugin.originalHidden = nil
            plugin.originalInset = nil
            plugin.originalStyle = nil
            plugin.originalIdleTimer = nil
            invoke.resolve()
        }
    }
}

@_cdecl("init_plugin_screen")
func initPlugin() -> Plugin { ScreenPlugin() }
