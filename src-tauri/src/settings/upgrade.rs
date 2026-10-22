use serde_json::Value;

use super::SettingsDocument;

/// 补齐缺省设置，保留用户值和未知字段。
pub fn upgrade_settings_document(document: &mut Value) {
    let legacy_theme = document["reader"]["theme"].as_str().map(str::to_owned);
    if let Some(theme) = legacy_theme.as_deref() {
        if document.get("theme").is_none() {
            document["theme"] = Value::from(if theme == "dark" { "dark" } else { "light" });
        }
        if document["reader"].get("themes").is_none() {
            let mut themes = serde_json::json!({ "light": "paper", "dark": "dark" });
            match theme {
                "dark" => themes["dark"] = Value::from("dark"),
                "sepia" => themes["light"] = Value::from("sepia"),
                "system" => themes["light"] = Value::from("white"),
                _ => {}
            }
            document["reader"]["themes"] = themes;
        }
    }
    let defaults =
        serde_json::to_value(SettingsDocument::default()).expect("serializable settings defaults");
    fill_missing(document, &defaults);
    if let Some(reader) = document.get_mut("reader").and_then(Value::as_object_mut) {
        for key in ["theme", "textColor", "backgroundColor"] {
            reader.remove(key);
        }
    }
}

fn fill_missing(value: &mut Value, defaults: &Value) {
    if let (Some(value), Some(defaults)) = (value.as_object_mut(), defaults.as_object()) {
        for (key, default) in defaults {
            match value.get_mut(key) {
                Some(current) => fill_missing(current, default),
                None => {
                    value.insert(key.clone(), default.clone());
                }
            }
        }
    }
}
