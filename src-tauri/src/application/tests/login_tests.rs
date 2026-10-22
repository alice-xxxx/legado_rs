use super::*;
use crate::application::ApplicationService;
use crate::source_catalog::SourceRecord;
use crate::source_engine::EngineFuture;
use crate::source_engine::SourceEngineRequest;
use crate::source_engine::SourceExecutor;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

struct LoginExecutor {
    requests: mpsc::UnboundedSender<SourceEngineRequest>,
    wait_for_submit: bool,
}

impl SourceExecutor for LoginExecutor {
    fn execute<'a>(&'a self, request: SourceEngineRequest) -> EngineFuture<'a> {
        Box::pin(async move {
            assert_eq!(request.operation, "login");
            self.requests.send(request.clone()).unwrap();
            if request.keyword.as_deref() == Some("submit") && self.wait_for_submit {
                std::future::pending::<()>().await;
            }
            Ok(json!({"hasLoginState": request.keyword.as_deref() == Some("submit")}))
        })
    }
}

async fn service(
    wait_for_submit: bool,
) -> (
    tempfile::TempDir,
    ApplicationService,
    mpsc::UnboundedReceiver<SourceEngineRequest>,
) {
    let directory = tempfile::tempdir().unwrap();
    let (requests, receiver) = mpsc::unbounded_channel();
    let service = ApplicationService::open_with_executor(
        directory.path(),
        Arc::new(LoginExecutor {
            requests,
            wait_for_submit,
        }),
    )
    .await
    .unwrap();
    service.bootstrap().await.unwrap();
    service
        .sources
        .replace(&[SourceRecord {
            id: "login-source".into(),
            name: "Test".into(),
            group: None,
            enabled: true,
            source: json!({"bookSourceUrl":"https://example.invalid", "bookSourceName":"Test"}),
        }])
        .await
        .unwrap();
    (directory, service, receiver)
}

#[tokio::test]
async fn form_and_button_submissions_use_one_engine_call_without_followup_queries() {
    let (_directory, service, mut requests) = service(false).await;
    let fields = HashMap::from([("account".into(), "reader".into())]);
    service
        .login("login-source", LoginAction::Open)
        .await
        .unwrap();
    service
        .login(
            "login-source",
            LoginAction::Submit {
                fields: fields.clone(),
                button: None,
            },
        )
        .await
        .unwrap();
    service
        .login(
            "login-source",
            LoginAction::Submit {
                fields: fields.clone(),
                button: Some(2),
            },
        )
        .await
        .unwrap();
    service
        .login("login-source", LoginAction::Clear)
        .await
        .unwrap();
    for (kind, button) in [
        ("open", None),
        ("submit", None),
        ("submit", Some(2)),
        ("clear", None),
    ] {
        let request = requests.recv().await.unwrap();
        assert_eq!(request.keyword.as_deref(), Some(kind));
        assert_eq!(request.action_id, button);
        if kind == "submit" {
            assert_eq!(request.credentials, Some(fields.clone()));
        }
    }
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_interrupts_pending_rules_and_reopening_replaces_the_session() {
    let (_directory, service, mut requests) = service(true).await;
    service
        .login("login-source", LoginAction::Open)
        .await
        .unwrap();
    let old = service.login.borrow().clone().unwrap();
    assert_eq!(old.id.len(), 32);
    requests.recv().await.unwrap();
    let submit = {
        let service = service.clone();
        tokio::spawn(async move {
            service
                .login(
                    "login-source",
                    LoginAction::Submit {
                        fields: HashMap::new(),
                        button: None,
                    },
                )
                .await
        })
    };
    let request = requests.recv().await.unwrap();
    service
        .login("other-source", LoginAction::Cancel)
        .await
        .unwrap();
    assert!(!old.cancel.is_cancelled());
    service
        .login("login-source", LoginAction::Cancel)
        .await
        .unwrap();
    assert!(request.cancellation.unwrap().is_cancelled());
    assert!(tokio::time::timeout(Duration::from_secs(1), submit)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    service
        .login("login-source", LoginAction::Open)
        .await
        .unwrap();
    let reopened = service.login.borrow().clone().unwrap();
    service
        .login("login-source", LoginAction::Open)
        .await
        .unwrap();
    assert!(reopened.cancel.is_cancelled());
    assert!(!service
        .login
        .borrow()
        .as_ref()
        .unwrap()
        .cancel
        .is_cancelled());
    service.cancel_login().await.unwrap();
}

#[test]
fn emitted_updates_keep_effects_and_strip_private_browser_data() {
    let result = sanitize_source_login_result(json!({
        "hasLoginState": true,
        "form": {"mode":"web", "message":"ignored"}, "webOpen": true,
        "web": {"url":"https://example.invalid/login", "cookieHeader":"secret=value"},
        "status":"executed", "code":"unused", "authenticated":null,
        "inputUpdates":{"account":"reader"}, "copyText":"code",
    }))
    .unwrap();
    assert_eq!(
        result,
        json!({
            "hasLoginState":true, "form":{"mode":"web"}, "webOpen":true,
            "inputUpdates":{"account":"reader"}, "copyText":"code",
        })
    );
}

#[cfg(test)]
mod login_form_tests {
    use super::*;

    #[test]
    fn forwards_source_ui_effects_and_restricts_refresh_targets() {
        let result = sanitize_source_login_result(json!({
            "hasLoginState": false,
            "messages": ["已保存设置"], "inputUpdates": {"设备ID": "123"},
            "refreshTargets": ["explore", "invalid"],
        }))
        .unwrap();
        assert_eq!(result["message"], json!("已保存设置"));
        assert_eq!(result["inputUpdates"]["设备ID"], "123");
        assert_eq!(result["refreshTargets"], json!(["explore"]));
    }

    fn form() -> Value {
        json!({
            "canLogin": true,
            "fields": [
                { "name": "账号", "type": "text" },
                { "name": "验证码", "type": "text" },
                { "name": "线路", "type": "select", "choices": ["一", "二"] },
                { "name": "记住", "type": "toggle" }
            ],
            "actions": [{ "id": 2, "label": "获取验证码" }],
            "layout": [
                { "kind": "title", "label": "登录说明", "cols": 1, "rows": 1 },
                { "kind": "field", "name": "账号", "cols": 1, "rows": 1 },
                { "kind": "action", "id": 2, "cols": 2, "rows": 1 },
                { "kind": "field", "name": "验证码", "cols": 2, "rows": 1 },
                { "kind": "field", "name": "线路", "cols": 2, "rows": 1 },
                { "kind": "field", "name": "记住", "cols": 2, "rows": 1 }
            ]
        })
    }

    #[test]
    fn preserves_row_order_titles_and_layout_without_exposing_actions() {
        let mut input = form();
        input["layout"][0]["style"] = json!({ "css": "unexpected" });
        input["actions"][0]["action"] = json!("private script");
        let result = sanitize_source_login_form(&input).unwrap();
        assert_eq!(result["layout"][0]["label"], "登录说明");
        assert_eq!(result["layout"][2]["kind"], "action");
        assert_eq!(result["layout"][3]["name"], "验证码");
        assert_eq!(result["layout"][3]["cols"], 2);
        assert!(result["layout"][0].get("style").is_none());
        assert!(result["actions"][0].get("action").is_none());
        assert_eq!(result["fields"][2]["choices"], json!(["一", "二"]));
    }

    #[test]
    fn supports_older_engines_without_layout() {
        let mut input = form();
        input.as_object_mut().unwrap().remove("layout");
        let result = sanitize_source_login_form(&input).unwrap();
        assert_eq!(result["layout"].as_array().unwrap().len(), 5);
        assert_eq!(result["layout"][0]["name"], "账号");
        assert_eq!(result["layout"][0]["cols"], 1);
        assert_eq!(result["layout"][4]["id"], 2);
    }

    #[test]
    fn rejects_broken_layout_references_and_dimensions() {
        for (key, value) in [("cols", json!(0)), ("cols", json!(5)), ("rows", json!(13))] {
            let mut input = form();
            input["layout"][0][key] = value;
            assert!(sanitize_source_login_form(&input).is_err());
        }
        let mut input = form();
        input["layout"][1]["name"] = json!("unknown");
        assert!(sanitize_source_login_form(&input).is_err());
        let mut input = form();
        input["layout"][2]["id"] = json!(99);
        assert!(sanitize_source_login_form(&input).is_err());
        let mut input = form();
        input["layout"][3]["name"] = json!("账号");
        assert!(sanitize_source_login_form(&input).is_err());
        let mut input = form();
        input["layout"].as_array_mut().unwrap().pop();
        assert!(sanitize_source_login_form(&input).is_err());
    }
}
