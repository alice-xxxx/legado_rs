#[cfg(target_os = "windows")]
pub fn set_gestures(webview: tauri::webview::PlatformWebview, enabled: bool) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings6;
    use windows_core::Interface;
    // Tauri runs this callback on the WebView's owning thread.
    unsafe {
        webview
            .controller()
            .CoreWebView2()
            .and_then(|view| view.Settings())
            .and_then(|settings| settings.cast::<ICoreWebView2Settings6>())
            .and_then(|settings| settings.SetIsSwipeNavigationEnabled(enabled))
    }
    .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
pub fn set_gestures(webview: tauri::webview::PlatformWebview, enabled: bool) -> Result<(), String> {
    // with_webview guarantees the WKWebView is alive and accessed on its UI thread.
    unsafe {
        let view = &*(webview.inner() as *const objc2_web_kit::WKWebView);
        view.setAllowsBackForwardNavigationGestures(enabled);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn set_gestures(webview: tauri::webview::PlatformWebview, enabled: bool) -> Result<(), String> {
    use webkit2gtk::{SettingsExt, WebViewExt};
    let settings = webview
        .inner()
        .settings()
        .ok_or("WebView settings are unavailable")?;
    settings.set_enable_back_forward_navigation_gestures(enabled);
    Ok(())
}
