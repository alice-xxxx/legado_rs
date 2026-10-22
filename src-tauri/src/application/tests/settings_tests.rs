use crate::application::ApplicationService;
use crate::source_engine::EngineFuture;
use crate::source_engine::SourceEngineRequest;
use crate::source_engine::SourceExecutor;
use serde_json::json;
use std::sync::Arc;

struct UnusedExecutor;
impl SourceExecutor for UnusedExecutor {
    fn execute<'a>(&'a self, _request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async { Err("Settings initialization must not execute a source".into()) })
    }
}

#[tokio::test]
async fn startup_persists_complete_settings_and_saves_both_reading_profiles() {
    let fresh = tempfile::tempdir().unwrap();
    let service = ApplicationService::open_with_executor(fresh.path(), Arc::new(UnusedExecutor))
        .await
        .unwrap();
    let settings = service
        .store
        .read_json_ref(&service.store.settings_ref())
        .await
        .unwrap();
    assert_eq!(settings["theme"], "system");
    assert_eq!(
        settings["reader"]["themes"],
        json!({"light":"paper","dark":"dark"})
    );
    assert_eq!(settings["reader"]["audioPlaybackRate"], 1.0);
    assert_eq!(settings["reader"]["audioSkipSeconds"], 15);

    let legacy = tempfile::tempdir().unwrap();
    let mut settings = serde_json::to_value(crate::models::SettingsDocument::default()).unwrap();
    settings.as_object_mut().unwrap().remove("theme");
    settings["reader"] = json!({"theme":"sepia","fontSizePx":25,"ttsRate":1.75});
    std::fs::write(
        legacy.path().join("settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let service = ApplicationService::open_with_executor(legacy.path(), Arc::new(UnusedExecutor))
        .await
        .unwrap();
    let mut persisted = service
        .store
        .read_json_ref(&service.store.settings_ref())
        .await
        .unwrap();
    assert_eq!(
        persisted["theme"], "light",
        "Settings remain in their existing JSON file"
    );
    assert_eq!(persisted["reader"]["themes"]["light"], "sepia");
    assert_eq!(persisted["reader"]["fontSizePx"], 25);
    persisted["reader"]["themes"]["light"] = json!("sepia");
    persisted["reader"]["fontSizePx"] = json!(25);
    assert_eq!(persisted["reader"]["audioSkipSeconds"], 15);
    persisted["theme"] = json!("dark");
    persisted["reader"]["themes"]["dark"] = json!("paper");
    service.save_settings(persisted).await.unwrap();
    let saved = service
        .store
        .read_json_ref(&service.store.settings_ref())
        .await
        .unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(
        saved["reader"]["themes"],
        json!({"light":"sepia","dark":"paper"})
    );
    assert_eq!(saved["reader"]["fontSizePx"], 25);
}
