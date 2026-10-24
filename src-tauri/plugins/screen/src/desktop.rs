use crate::{ScreenOptions, ScreenUpdate};
use std::{collections::HashMap, sync::Mutex};
use tauri::{Manager, Runtime, Webview};

mod awake;

#[derive(Default)]
pub struct WindowState {
    fullscreen: Option<bool>,
    awake: Option<awake::Guard>,
}
#[derive(Default)]
pub struct State(pub Mutex<HashMap<String, WindowState>>);

pub fn apply<R: Runtime>(
    webview: &Webview<R>,
    options: ScreenOptions,
) -> Result<ScreenUpdate, String> {
    let state = webview.state::<State>();
    let mut windows = state.0.lock().map_err(|error| error.to_string())?;
    let current = windows.entry(webview.window().label().into()).or_default();
    let mut result = ScreenUpdate::default();
    if let Some(enabled) = options.fullscreen {
        if current.fullscreen.is_none() {
            current.fullscreen = Some(
                webview
                    .window()
                    .is_fullscreen()
                    .map_err(|error| error.to_string())?,
            );
        }
        webview
            .window()
            .set_fullscreen(enabled)
            .map_err(|error| error.to_string())?;
        result.applied.push("fullscreen".into());
    }
    if let Some(enabled) = options.keep_awake {
        if enabled && current.awake.is_none() {
            current.awake = Some(awake::Guard::new()?);
        }
        if !enabled {
            current.awake = None;
        }
        result.applied.push("keepAwake".into());
    }
    for (name, requested) in [
        ("systemUiHidden", options.system_ui_hidden.is_some()),
        ("systemUiStyle", options.system_ui_style.is_some()),
        ("orientation", options.orientation.is_some()),
        ("brightness", options.brightness.is_some()),
    ] {
        if requested {
            result.unsupported.push(name.into());
        }
    }
    Ok(result)
}

pub fn get_info<R: Runtime>(webview: &Webview<R>) -> Result<serde_json::Value, String> {
    let size = webview
        .window()
        .inner_size()
        .map_err(|error| error.to_string())?;
    let scale = webview
        .window()
        .scale_factor()
        .map_err(|error| error.to_string())?;
    let fullscreen = webview
        .window()
        .is_fullscreen()
        .map_err(|error| error.to_string())?;
    let state = webview.state::<State>();
    let windows = state.0.lock().map_err(|error| error.to_string())?;
    let awake = windows
        .get(webview.window().label())
        .is_some_and(|state| state.awake.is_some());
    Ok(serde_json::json!({
        "width": size.width as f64 / scale, "height": size.height as f64 / scale, "scale": scale,
        "fullscreen": fullscreen, "keepAwake": awake, "brightness": null, "systemUiHidden": null, "systemUiStyle": null,
        "orientation": if size.width > size.height { "landscape" } else { "portrait" },
        "safeArea": {"top": 0, "right": 0, "bottom": 0, "left": 0},
        "capabilities": {"fullscreen": true, "keepAwake": true, "brightness": false,
            "systemUiHidden": false, "systemUiStyle": false, "orientation": false}
    }))
}

pub fn reset<R: Runtime>(webview: &Webview<R>) -> Result<(), String> {
    let state = webview.state::<State>();
    let mut windows = state.0.lock().map_err(|error| error.to_string())?;
    if let Some(current) = windows.get_mut(webview.window().label()) {
        current.awake = None;
        if let Some(original) = current.fullscreen {
            webview
                .window()
                .set_fullscreen(original)
                .map_err(|error| error.to_string())?;
        }
        windows.remove(webview.window().label());
    }
    Ok(())
}
