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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fresh_settings_publish_both_reading_themes_and_all_required_defaults() {
        let settings = serde_json::to_value(SettingsDocument::default()).unwrap();
        assert_eq!(settings["theme"], "system");
        assert_eq!(
            settings["reader"]["themes"],
            json!({"light":"paper","dark":"dark"})
        );
        assert_eq!(settings["reader"]["ttsRate"], 1.0);
        assert_eq!(settings["reader"]["audioSkipSeconds"], 15);
        assert_eq!(settings["reader"]["readingMode"], "cover");
        for old in ["theme", "textColor", "backgroundColor"] {
            assert!(settings["reader"].get(old).is_none());
        }
    }

    #[test]
    fn upgrades_legacy_settings_without_overwriting_user_values() {
        for (old, app, reading) in [
            ("paper", "light", "paper"),
            ("sepia", "light", "sepia"),
            ("dark", "dark", "dark"),
            ("system", "light", "white"),
        ] {
            let mut settings = json!({"schemaVersion":1,"reader":{"theme":old,"fontSizePx":25,"ttsRate":1.75,
                "textColor":"#123456","backgroundColor":"#ffffff","futureReaderSetting":true},
                "home":{"tabs":[{"id":"custom","title":"我的主页","sortOrder":0,"sections":[]}]},
                "lastBackupAtMs":123,"futureSetting":42});
            upgrade_settings_document(&mut settings);
            assert_eq!(settings["theme"], app);
            assert_eq!(settings["reader"]["themes"][app], reading);
            assert_eq!(settings["reader"]["fontSizePx"], 25);
            assert_eq!(settings["reader"]["ttsRate"], 1.75);
            assert_eq!(settings["reader"]["audioPlaybackRate"], 1.0);
            assert_eq!(settings["reader"]["audioSkipSeconds"], 15);
            assert_eq!(settings["reader"]["readingMode"], "cover");
            assert_eq!(settings["home"]["tabs"][0]["id"], "custom");
            assert_eq!(settings["lastBackupAtMs"], 123);
            assert_eq!(settings["futureSetting"], 42);
            assert_eq!(settings["reader"]["futureReaderSetting"], true);
            serde_json::from_value::<SettingsDocument>(settings.clone()).unwrap();
            let once = settings.clone();
            upgrade_settings_document(&mut settings);
            assert_eq!(settings, once);
        }
    }

    #[test]
    fn preserves_current_profiles_and_rejects_invalid_theme_values() {
        let mut settings = serde_json::to_value(SettingsDocument::default()).unwrap();
        settings["theme"] = json!("dark");
        settings["reader"]["themes"] = json!({"light":"sepia","dark":"paper"});
        upgrade_settings_document(&mut settings);
        assert_eq!(
            settings["reader"]["themes"],
            json!({"light":"sepia","dark":"paper"})
        );
        settings["reader"]["themes"]["dark"] = json!("system");
        assert!(serde_json::from_value::<SettingsDocument>(settings.clone()).is_err());
        settings["reader"]["themes"]["dark"] = json!("dark");
        settings["theme"] = json!("paper");
        assert!(serde_json::from_value::<SettingsDocument>(settings).is_err());
    }

    #[test]
    fn rejects_unknown_unified_reading_modes() {
        let mut settings = serde_json::to_value(SettingsDocument::default()).unwrap();
        settings["reader"]["readingMode"] = json!("diagonal");
        assert!(serde_json::from_value::<SettingsDocument>(settings).is_err());
    }
}
