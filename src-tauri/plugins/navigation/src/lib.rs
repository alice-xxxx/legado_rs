//! Native navigation input. Page history and back actions belong to the consumer.
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_navigation);

#[cfg(target_os = "ios")]
struct Navigation<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[tauri::command]
async fn set_gestures<R: Runtime>(
    webview: tauri::Webview<R>,
    enabled: bool,
    local: bool,
) -> Result<bool, String> {
    #[cfg(target_os = "ios")]
    {
        use tauri::Manager;
        webview
            .state::<Navigation<R>>()
            .0
            .run_mobile_plugin_async::<serde_json::Value>(
                "setNavigationGestures",
                serde_json::json!({"enabled": enabled, "local": local}),
            )
            .await
            .map(|_| true)
            .map_err(|error| error.to_string())
    }
    #[cfg(target_os = "android")]
    {
        // Android's system Back is delivered by the Tauri core listener in guest-js.
        let _ = (webview, enabled, local);
        Ok(false)
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        webview
            .with_webview(move |native| {
                let _ = sender.send(desktop::set_gestures(native, enabled && !local));
            })
            .map_err(|error| error.to_string())?;
        receiver
            .await
            .map_err(|error| error.to_string())?
            .map(|_| true)
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("navigation")
        .invoke_handler(tauri::generate_handler![set_gestures])
        .setup(|app, api| {
            #[cfg(target_os = "ios")]
            {
                use tauri::Manager;
                app.manage(Navigation(api.register_ios_plugin(init_plugin_navigation)?));
            }
            #[cfg(not(target_os = "ios"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}
