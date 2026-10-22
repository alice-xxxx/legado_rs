//! 词典页面暂不提供本地资源。
#[tauri::command]
pub async fn lookup_dictionary(
    word: String,
    language: String,
    service: tauri::State<'_, crate::application::ApplicationService>,
) -> Result<serde_json::Value, String> {
    let _ = (word, language, service);
    Err("词典页面暂未实现".into())
}
