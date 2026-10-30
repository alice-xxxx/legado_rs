use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Runtime,
};

struct Installer<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("update-installer")
        .setup(|app, api| {
            app.manage(Installer(api.register_android_plugin(
                "com.xyz.foliver.updates",
                "UpdateInstallerPlugin",
            )?));
            Ok(())
        })
        .build()
}

pub async fn install<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<bool, String> {
    let result = app
        .state::<Installer<R>>()
        .0
        .run_mobile_plugin_async::<serde_json::Value>("install", ())
        .await
        .map_err(|error| error.to_string())?;
    Ok(result["permissionRequired"].as_bool().unwrap_or(false))
}
