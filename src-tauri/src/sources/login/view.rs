//! 登录界面的数据契约，只输出展示字段，不输出规则、Cookie 或原生会话。

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use tauri_plugin_source_engine::{LoginForm, LoginState};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Update {
    pub source_id: String,
    pub has_login_state: bool,
    pub web_open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<LoginForm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub input_updates: HashMap<String, String>,
    pub refresh_targets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy_text: Option<String>,
}
pub(super) fn credentials(fields: &HashMap<String, String>) -> Result<(), String> {
    if fields.len() > 64
        || fields.iter().any(|(name, value)| {
            name.is_empty()
                || name.len() > 128
                || name.chars().any(char::is_control)
                || value.len() > 16 * 1024
        })
        || fields
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>()
            > 64 * 1024
    {
        return Err("登录字段超过限制".into());
    }
    Ok(())
}
fn form(value: &mut LoginForm) -> Result<(), String> {
    if !matches!(value.mode.as_str(), "form" | "web") {
        return Err("登录表单模式无效".into());
    }
    if value.fields.len() > 64 || value.actions.len() > 256 || value.layout.len() > 320 {
        return Err("登录表单超过大小限制".into());
    }
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for field in &mut value.fields {
        if field.name.is_empty()
            || field.name.len() > 128
            || field.name.chars().any(char::is_control)
            || !names.insert(field.name.clone())
            || !matches!(
                field.r#type.as_str(),
                "text" | "password" | "select" | "toggle"
            )
            || field.choices.len() > 64
            || field.choices.iter().any(|v| v.len() > 256)
        {
            return Err("登录表单字段无效".into());
        }
        field.password = field.password || field.r#type == "password";
        if field.password {
            field.initial_value = None;
        }
        if field
            .initial_value
            .as_ref()
            .is_some_and(|value| value.len() > 16 * 1024)
        {
            return Err("登录字段默认值过长".into());
        }
        if field.label.is_empty() {
            field.label = field.name.clone();
        }
        field.label = field.label.chars().take(128).collect();
    }
    for action in &value.actions {
        if action.id > i32::MAX as usize
            || !ids.insert(action.id)
            || action.label.is_empty()
            || action.label.chars().count() > 128
        {
            return Err("登录表单按钮无效".into());
        }
    }
    let mut placed_names = HashSet::new();
    let mut placed_ids = HashSet::new();
    for cell in &value.layout {
        let valid = match cell.kind.as_str() {
            "field" => {
                let name_valid = cell.name.as_ref().is_some_and(|name| {
                    placed_names.insert(name.clone());
                    names.contains(name)
                });
                let id_valid = cell.id.is_none_or(|id| {
                    id <= i32::MAX as usize && ids.insert(id) && placed_ids.insert(id)
                });
                name_valid && id_valid
            }
            "action" => cell
                .id
                .is_some_and(|id| ids.contains(&id) && placed_ids.insert(id)),
            "title" => cell.label.as_ref().is_some_and(|label| label.len() <= 8192),
            _ => false,
        };
        if !valid || !(1..=4).contains(&cell.cols) || !(1..=12).contains(&cell.rows) {
            return Err("登录表单布局无效".into());
        }
    }
    if !value.layout.is_empty()
        && (placed_names.len() != names.len() || placed_ids.len() != ids.len())
    {
        return Err("登录表单布局不完整".into());
    }
    Ok(())
}
pub(super) fn update(
    source_id: &str,
    mut state: LoginState,
    web_open: bool,
    passwords: &mut HashSet<String>,
) -> Result<Update, String> {
    if let Some(value) = &mut state.form {
        form(value)?;
        passwords.extend(
            value
                .fields
                .iter()
                .filter(|f| f.password)
                .map(|f| f.name.clone()),
        );
    }
    credentials(&state.input_updates)?;
    state
        .input_updates
        .retain(|name, _| !passwords.contains(name));
    if state
        .copy_text
        .as_ref()
        .is_some_and(|v| v.len() > 1024 * 1024)
    {
        return Err("登录复制内容超过限制".into());
    }
    state
        .refresh_targets
        .retain(|v| matches!(v.as_str(), "login" | "explore" | "book"));
    state.refresh_targets.sort();
    state.refresh_targets.dedup();
    Ok(Update {
        source_id: source_id.into(),
        has_login_state: state.has_login_state,
        web_open,
        form: state.form,
        message: state
            .messages
            .pop()
            .or(state.message)
            .map(|v| v.chars().take(4096).collect()),
        error: state.error.map(|v| v.chars().take(4096).collect()),
        input_updates: state.input_updates,
        refresh_targets: state.refresh_targets,
        copy_text: state.copy_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_source_engine::{LoginField, LoginWeb};
    #[test]
    fn passwords_and_native_credentials_never_enter_updates() {
        let mut passwords = HashSet::new();
        let first = LoginState {
            form: Some(LoginForm {
                mode: "form".into(),
                fields: vec![LoginField {
                    name: "密码".into(),
                    r#type: "password".into(),
                    initial_value: Some("saved-secret".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let first = update("source", first, false, &mut passwords).unwrap();
        assert!(!serde_json::to_string(&first)
            .unwrap()
            .contains("saved-secret"));
        let later = LoginState {
            web: Some(LoginWeb {
                url: "https://private.example".into(),
                cookie_header: "session=secret".into(),
            }),
            input_updates: HashMap::from([
                ("密码".into(), "秘密".into()),
                ("账号".into(), "用户".into()),
            ]),
            ..Default::default()
        };
        let later = update("source", later, false, &mut passwords).unwrap();
        assert_eq!(
            later.input_updates.get("账号").map(String::as_str),
            Some("用户")
        );
        let output = serde_json::to_string(&later).unwrap();
        for private in ["秘密", "session=secret", "private.example"] {
            assert!(!output.contains(private));
        }
    }
    #[test]
    fn repeated_fields_and_large_forms_keep_their_action_ids() {
        use tauri_plugin_source_engine::{LoginButton, LoginLayout};
        let mut value = LoginForm {
            mode: "form".into(),
            fields: vec![LoginField {
                name: "shared".into(),
                r#type: "text".into(),
                ..Default::default()
            }],
            actions: (2..68)
                .map(|id| LoginButton {
                    id,
                    label: format!("action {id}"),
                })
                .collect(),
            layout: (0..2)
                .map(|id| LoginLayout {
                    kind: "field".into(),
                    name: Some("shared".into()),
                    id: Some(id),
                    cols: 1,
                    rows: 1,
                    label: None,
                })
                .chain((2..68).map(|id| LoginLayout {
                    kind: "action".into(),
                    id: Some(id),
                    cols: 2,
                    rows: 1,
                    name: None,
                    label: None,
                }))
                .collect(),
            ..Default::default()
        };
        form(&mut value).unwrap();
        assert_eq!(value.layout.len(), 68);
        value.layout[1].id = Some(2);
        assert!(form(&mut value).is_err());
    }
}
