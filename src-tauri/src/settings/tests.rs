use serde_json::json;

async fn open(root: &std::path::Path) -> (crate::database::Database, crate::resources::Resources) {
    let database = crate::database::Database::new(root).unwrap();
    let resources = crate::resources::Resources::open(root, database.clone())
        .await
        .unwrap();
    (database, resources)
}

#[tokio::test]
async fn startup_persists_complete_settings_and_saves_both_reading_profiles() {
    let fresh = tempfile::tempdir().unwrap();
    let (_, resources) = open(fresh.path()).await;
    let settings = resources.settings.read().await.unwrap();
    assert_eq!(settings["theme"], "system");
    assert_eq!(
        settings["reader"]["themes"],
        json!({"light":"paper","dark":"dark"})
    );
    assert_eq!(settings["reader"]["audioPlaybackRate"], 1.0);
    assert_eq!(settings["reader"]["audioSkipSeconds"], 15);

    let legacy = tempfile::tempdir().unwrap();
    let mut settings = serde_json::to_value(crate::settings::SettingsDocument::default()).unwrap();
    settings.as_object_mut().unwrap().remove("theme");
    settings["reader"] = json!({"theme":"sepia","fontSizePx":25,"ttsRate":1.75});
    std::fs::write(
        legacy.path().join("settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let (database, resources) = open(legacy.path()).await;
    let mut persisted = resources.settings.read().await.unwrap();
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
    super::save(&resources, &database, persisted).await.unwrap();
    let saved = resources.settings.read().await.unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(
        saved["reader"]["themes"],
        json!({"light":"sepia","dark":"paper"})
    );
    assert_eq!(saved["reader"]["fontSizePx"], 25);
}

#[tokio::test]
async fn rejected_settings_leave_the_file_and_backup_time_intact() {
    let root = tempfile::tempdir().unwrap();
    let (database, resources) = open(root.path()).await;
    resources
        .settings
        .update(|mut current| {
            current["lastBackupAtMs"] = json!(123);
            Ok(current)
        })
        .await
        .unwrap();
    let mut document = resources.settings.read().await.unwrap();
    document["lastBackupAtMs"] = json!(999);
    document["futureSetting"] = json!("preserved");
    let saved = super::save(&resources, &database, document).await.unwrap();
    assert_eq!(saved["lastBackupAtMs"], 123);
    assert_eq!(saved["futureSetting"], "preserved");
    let mut invalid = saved.clone();
    invalid["reader"]["fontSizePx"] = json!(99);
    assert!(super::save(&resources, &database, invalid).await.is_err());
    assert_eq!(resources.settings.read().await.unwrap(), saved);
    let mut invalid_home = saved.clone();
    invalid_home["home"]["tabs"][0]["sections"] = json!([{"id":"section","title":"未登记来源","sourceId":"missing","categoryId":"missing","style":0,"sortOrder":0}]);
    assert!(super::save(&resources, &database, invalid_home)
        .await
        .is_err());
    assert_eq!(resources.settings.read().await.unwrap(), saved);
}
