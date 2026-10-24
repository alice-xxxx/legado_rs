use super::*;

#[tauri::command]
pub async fn apply<R: Runtime>(
    webview: Webview<R>,
    options: ScreenOptions,
) -> Result<ScreenUpdate, String> {
    if options
        .brightness
        .is_some_and(|value| !value.is_finite() || value != -1.0 && !(0.0..=1.0).contains(&value))
    {
        return Err("Brightness must be between 0 and 1, or -1 to restore it".into());
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        webview
            .state::<Screen<R>>()
            .0
            .run_mobile_plugin_async("apply", serde_json::json!({"options": options}))
            .await
            .map_err(|error| error.to_string())
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        desktop::apply(&webview, options)
    }
}

#[tauri::command]
pub async fn get_info<R: Runtime>(webview: Webview<R>) -> Result<serde_json::Value, String> {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        webview
            .state::<Screen<R>>()
            .0
            .run_mobile_plugin_async("getInfo", serde_json::json!({}))
            .await
            .map_err(|error| error.to_string())
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        desktop::get_info(&webview)
    }
}

/// Restore controls before a WebView reload or when its consumer is disposed.
#[tauri::command]
pub async fn reset<R: Runtime>(webview: Webview<R>) -> Result<(), String> {
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        webview
            .state::<Screen<R>>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>("reset", serde_json::json!({}))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        desktop::reset(&webview)
    }
}
