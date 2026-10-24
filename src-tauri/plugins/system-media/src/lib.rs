//! System controls only. Media playback belongs to the WebView or system TTS.
use tauri::{plugin::{Builder, TauriPlugin}, Runtime};

#[cfg(mobile)]
mod mobile {
    use super::*;
    use tauri::Manager;
    struct Session<R: Runtime>(tauri::plugin::PluginHandle<R>);
    #[cfg(target_os = "ios")]
    tauri::ios_plugin_binding!(init_plugin_system_media);

    #[tauri::command]
    async fn update<R: Runtime>(app: tauri::AppHandle<R>, state: serde_json::Value) -> Result<(), String> {
        app.state::<Session<R>>().0.run_mobile_plugin_async::<serde_json::Value>("update", serde_json::json!({"state": state}))
            .await.map(|_| ()).map_err(|error| error.to_string())
    }
    #[tauri::command]
    async fn clear<R: Runtime>(app: tauri::AppHandle<R>, id: u64) -> Result<(), String> {
        app.state::<Session<R>>().0.run_mobile_plugin_async::<serde_json::Value>("clear", serde_json::json!({"id": id}))
            .await.map(|_| ()).map_err(|error| error.to_string())
    }
    pub fn init<R: Runtime>() -> TauriPlugin<R> {
        Builder::new("system-media")
            .invoke_handler(tauri::generate_handler![update, clear])
            .setup(|app, api| {
                #[cfg(target_os = "android")]
                let handle = api.register_android_plugin("io.legado.media", "SystemMediaPlugin")?;
                #[cfg(target_os = "ios")]
                let handle = api.register_ios_plugin(init_plugin_system_media)?;
                app.manage(Session(handle));
                Ok(())
            }).build()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    #[cfg(mobile)]
    { mobile::init() }
    #[cfg(not(mobile))]
    { Builder::new("system-media").build() }
}
