//! Window-scoped screen controls. Reader and video policy belong to the consumer.
use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime, Webview,
};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod desktop;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_screen);

#[cfg(any(target_os = "ios", target_os = "android"))]
struct Screen<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    Portrait,
    Landscape,
    PortraitPrimary,
    PortraitSecondary,
    LandscapePrimary,
    LandscapeSecondary,
    Auto,
    Restore,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SystemUiStyle {
    Auto,
    Light,
    Dark,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct ScreenOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fullscreen: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_ui_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_ui_style: Option<SystemUiStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Orientation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_awake: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f64>,
}

#[derive(Default, Deserialize, Serialize)]
pub struct ScreenUpdate {
    pub applied: Vec<String>,
    pub unsupported: Vec<String>,
}

mod commands;
pub use commands::{apply, get_info, reset};

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("screen")
        .invoke_handler(tauri::generate_handler![
            commands::apply,
            commands::get_info,
            commands::reset
        ])
        .setup(|app, api| {
            #[cfg(target_os = "ios")]
            app.manage(Screen(api.register_ios_plugin(init_plugin_screen)?));
            #[cfg(target_os = "android")]
            app.manage(Screen(api.register_android_plugin(
                "com.xyz.foliver.screen",
                "ScreenPlugin",
            )?));
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            {
                let _ = api;
                app.manage(desktop::State::default());
            }
            Ok(())
        })
        .on_event(|app, event| {
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = event
            {
                if let Ok(mut state) = app.state::<desktop::State>().0.lock() {
                    state.remove(label);
                }
            }
            #[cfg(any(target_os = "ios", target_os = "android"))]
            let _ = (app, event);
        })
        .build()
}
