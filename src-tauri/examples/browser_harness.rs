//! Test-only HTTP adapter for running the real resource-oriented app service in Chromium.
//!
//! The source definition is sent by the Node test runner directly to this process. The browser
//! sees only normal app commands, processed JSON, and the real loopback resource server.

use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{HeaderValue, Method, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use legado_lib::application::{ApplicationService, PickerFile};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone)]
struct HarnessState {
    app: Arc<ApplicationService>,
    events: EventLog,
    next_listener_id: Arc<AtomicU64>,
}

#[derive(Clone, Default)]
struct EventLog {
    records: Arc<Mutex<VecDeque<EventRecord>>>,
    next_event_id: Arc<AtomicU64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventRecord {
    id: u64,
    event: String,
    payload: Value,
}

impl EventLog {
    fn emit(&self, event: &str, payload: Value) {
        let id = self.next_event_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut records = self.records.lock().expect("browser harness event lock");
        records.push_back(EventRecord {
            id,
            event: event.to_owned(),
            payload,
        });
        while records.len() > 512 {
            records.pop_front();
        }
    }
}

impl HarnessState {
    fn emit(&self, event: &str, payload: Value) {
        self.events.emit(event, payload);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvokeRequest {
    command: String,
    #[serde(default)]
    args: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupSourceRequest {
    source_json: String,
}

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = std::env::var_os("LEGADO_BROWSER_HARNESS_DATA")
        .map(PathBuf::from)
        .ok_or("LEGADO_BROWSER_HARNESS_DATA must point to a fresh temporary directory")?;
    let resource_dir = std::env::var_os("LEGADO_SOURCE_ENGINE_RESOURCE_DIR").map(PathBuf::from);
    let app = Arc::new(ApplicationService::open(data_dir, resource_dir).await?);
    let event_log = EventLog::default();
    let task_events = event_log.clone();
    app.set_task_notifier(Arc::new(move |payload| {
        task_events.emit("task-updated", payload);
    }));
    let state = HarnessState {
        app,
        events: event_log,
        next_listener_id: Arc::new(AtomicU64::new(0)),
    };
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).await?;
    let address = listener.local_addr()?;
    let resource_url = state.app.resource_server().base_url().to_owned();
    let router = Router::new()
        .route("/healthz", get(health))
        .route("/setup/source", post(import_test_source))
        .route("/invoke", post(invoke))
        .route("/events", get(events))
        .with_state(state)
        .layer(middleware::from_fn(browser_cors));

    println!("BROWSER_HARNESS_READY=http://{address}");
    println!("BROWSER_HARNESS_RESOURCE_URL={resource_url}");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn health(State(state): State<HarnessState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "resourceServerUrl": state.app.resource_server().base_url(),
    }))
}

async fn import_test_source(
    State(state): State<HarnessState>,
    Json(request): Json<SetupSourceRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .app
        .import_sources(&request.source_json)
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn invoke(
    State(state): State<HarnessState>,
    Json(request): Json<InvokeRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let args = request.args;
    let value = match request.command.as_str() {
        "plugin:event|listen" => Ok(json!(
            state.next_listener_id.fetch_add(1, Ordering::Relaxed) + 1
        )),
        "plugin:event|unlisten" | "plugin:event|emit" | "plugin:event|emit_to" => Ok(Value::Null),
        "app_bootstrap" => state.app.bootstrap().await.map_err(internal_error),
        "get_search_history" => state.app.get_search_history().await.map_err(internal_error),
        "delete_search_history" => {
            let resource = state
                .app
                .delete_search_history(string_arg(&args, "query")?)
                .await
                .map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "searchHistory", "resource": resource }),
            );
            Ok(resource)
        }
        "clear_search_history" => {
            let resource = state.app.clear_search_history().await.map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "searchHistory", "resource": resource }),
            );
            Ok(resource)
        }
        "list_sources" => state.app.list_sources().await.map_err(internal_error),
        "import_sources" => {
            let source_json = string_arg(&args, "sourceJson")?;
            let result = state
                .app
                .import_sources(source_json)
                .await
                .map_err(internal_error)?;
            state.emit("sources-updated", result["sources"].clone());
            Ok(result)
        }
        "import_sources_from_picker" => {
            let Some(path) = selected_path("LEGADO_BROWSER_HARNESS_SOURCE_FILE") else {
                return Ok(Json(json!({ "ok": true, "value": { "cancelled": true } })));
            };
            let source_json = tokio::fs::read_to_string(path).await.map_err(|error| {
                internal_error(format!("Cannot read selected source file: {error}"))
            })?;
            let result = state
                .app
                .import_sources(&source_json)
                .await
                .map_err(internal_error)?;
            state.emit("sources-updated", result["sources"].clone());
            Ok(result)
        }
        "remove_sources" => {
            let source_ids = string_array_arg(&args, "sourceIds")?;
            let result = state
                .app
                .remove_sources(&source_ids)
                .await
                .map_err(internal_error)?;
            state.emit("sources-updated", result["sources"].clone());
            Ok(result)
        }
        "update_source" => {
            let source_id = string_arg(&args, "sourceId")?;
            let patch = args.get("patch").cloned().unwrap_or(Value::Null);
            let result = state
                .app
                .update_source(source_id, patch)
                .await
                .map_err(internal_error)?;
            state.emit("sources-updated", result["sources"].clone());
            Ok(result)
        }
        "search_books" => {
            let source_ids = string_array_arg(&args, "sourceIds")?;
            let keyword = string_arg(&args, "keyword")?;
            let page = args.get("page").and_then(Value::as_u64).unwrap_or(1) as u32;
            let mut progress = Vec::new();
            let result = state
                .app
                .search_books(&source_ids, keyword, page, |event| progress.push(event))
                .await;
            for event in progress {
                state.emit("search-progress", event);
            }
            if let Ok(result) = &result {
                state.emit(
                    "search-complete",
                    json!({
                        "keyword": keyword,
                        "resource": result["resource"],
                        "bookCount": result["bookCount"],
                        "errors": result["errors"],
                    }),
                );
            }
            result.map_err(internal_error)
        }
        "start_search" => {
            let source_ids = string_array_arg(&args, "sourceIds")?;
            let keyword = string_arg(&args, "keyword")?;
            let page = args.get("page").and_then(Value::as_u64).unwrap_or(1) as u32;
            let result = state
                .app
                .start_search(&source_ids, keyword, page)
                .await
                .map_err(internal_error)?;
            state.emit("search-started", result.clone());
            let history = state.app.get_search_history().await.map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "searchHistory", "resource": history }),
            );
            Ok(result)
        }
        "search_book_source_candidates" => {
            let source_ids = string_array_arg(&args, "sourceIds")?;
            let keyword = args.get("keyword").and_then(Value::as_str);
            let page = args.get("page").and_then(Value::as_u64).unwrap_or(1) as u32;
            let result = state
                .app
                .search_book_source_candidates(
                    string_arg(&args, "bookId")?,
                    &source_ids,
                    keyword,
                    page,
                )
                .await
                .map_err(internal_error)?;
            state.emit("search-started", result.clone());
            Ok(result)
        }
        "tasks_resource" => state.app.tasks_resource().await.map_err(internal_error),
        "start_chapter_download" => {
            let book_id = string_arg(&args, "bookId")?;
            let from_index = usize_arg(&args, "fromIndex")?;
            let count = usize_arg(&args, "count")?;
            let result = state
                .app
                .start_chapter_download(book_id, from_index, count)
                .await
                .map_err(internal_error)?;
            state.emit(
                "task-updated",
                json!({ "task": result["task"], "resource": result["resource"] }),
            );
            Ok(result)
        }
        "refresh_chapters" | "check_new_chapters" => {
            let book_id = string_arg(&args, "bookId")?;
            let result = if request.command == "refresh_chapters" {
                state.app.refresh_chapters(book_id).await
            } else {
                state.app.check_new_chapters(book_id).await
            }
            .map_err(internal_error)?;
            state.emit(
                "task-updated",
                json!({ "task": result["task"], "resource": result["resource"] }),
            );
            Ok(result)
        }
        "pause_task" | "resume_task" | "cancel_task" => {
            let task_id = string_arg(&args, "taskId")?;
            match request.command.as_str() {
                "pause_task" => state.app.pause_task(task_id).await,
                "resume_task" => state.app.resume_task(task_id).await,
                _ => state.app.cancel_task(task_id).await,
            }
            .map_err(internal_error)
        }
        "add_book" => {
            let result = state
                .app
                .add_book(string_arg(&args, "resultId")?)
                .await
                .map_err(internal_error)?;
            state.emit("book-added", result["book"].clone());
            state.emit("shelf-updated", result["shelf"].clone());
            Ok(result)
        }
        "change_book_source" => {
            let book_id = string_arg(&args, "bookId")?;
            let result = state
                .app
                .change_book_source(
                    book_id,
                    string_arg(&args, "resultId")?,
                    typed_arg(&args, "confirmMissingAuthor")?,
                )
                .await
                .map_err(internal_error)?;
            state.emit("book-source-changed", result.clone());
            state.emit("shelf-updated", result["shelf"].clone());
            state.emit(
                "progress-saved",
                json!({ "bookId": book_id, "book": result["book"] }),
            );
            state.emit(
                "resource-updated",
                json!({ "kind": "bookmarks", "resource": result["bookmarks"]["resource"] }),
            );
            Ok(result)
        }
        "import_book_from_picker" => {
            let Some(path) = selected_path("LEGADO_BROWSER_HARNESS_BOOK_FILE") else {
                return Ok(Json(json!({ "ok": true, "value": { "cancelled": true } })));
            };
            let options = if args.get("options").is_some() {
                typed_arg::<legado_lib::local_books::LocalImportOptions>(&args, "options")?
            } else {
                legado_lib::local_books::LocalImportOptions::default()
            };
            let result = state
                .app
                .import_local_book_with_challenge(PickerFile::from_existing_path(path), options)
                .await
                .map_err(internal_error)?;
            if result.get("passwordRequired").and_then(Value::as_bool) == Some(true) {
                return Ok(Json(json!({ "ok": true, "value": result })));
            }
            state.emit("book-added", result["book"].clone());
            state.emit("shelf-updated", result["shelf"].clone());
            Ok(result)
        }
        "import_protected_pdf" => {
            let result = state
                .app
                .retry_pending_pdf_import(
                    string_arg(&args, "importToken")?,
                    string_arg(&args, "password")?.to_owned(),
                )
                .await
                .map_err(internal_error)?;
            state.emit("book-added", result["book"].clone());
            state.emit("shelf-updated", result["shelf"].clone());
            Ok(result)
        }
        "cancel_pending_pdf_import" => {
            let had_pending_import = state
                .app
                .cancel_pending_pdf_import(string_arg(&args, "importToken")?)
                .await
                .map_err(internal_error)?;
            Ok(json!({
                "cancelled": true,
                "hadPendingImport": had_pending_import,
            }))
        }
        "get_book" => state
            .app
            .get_book(string_arg(&args, "bookId")?)
            .await
            .map_err(internal_error),
        "prepare_chapters" => {
            let book_id = string_arg(&args, "bookId")?;
            let from_index = args.get("fromIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
            let count = args.get("count").and_then(Value::as_u64).unwrap_or(5) as usize;
            let result = state.app.prepare_chapters(book_id, from_index, count).await;
            if let Ok(result) = &result {
                state.emit(
                    "chapters-prepared",
                    json!({
                        "bookId": book_id,
                        "fromIndex": from_index,
                        "prepared": result["prepared"],
                        "book": result["book"],
                    }),
                );
            }
            result.map_err(internal_error)
        }
        "remove_book" => {
            let result = state
                .app
                .remove_book(string_arg(&args, "bookId")?)
                .await
                .map_err(internal_error)?;
            state.emit("shelf-updated", result["shelf"].clone());
            Ok(result)
        }
        "save_progress" => {
            let book_id = string_arg(&args, "bookId")?;
            let progress = args.get("progress").cloned().unwrap_or(Value::Null);
            let result = state
                .app
                .save_progress(book_id, progress)
                .await
                .map_err(internal_error)?;
            state.emit(
                "progress-saved",
                json!({ "bookId": book_id, "book": result }),
            );
            state.emit(
                "shelf-updated",
                state
                    .app
                    .resource_descriptor(&state.app.resource_store().shelf_ref()),
            );
            Ok(result)
        }
        "save_settings" => {
            let settings = args.get("settings").cloned().unwrap_or(Value::Null);
            let result = state
                .app
                .save_settings(settings)
                .await
                .map_err(internal_error)?;
            state.emit("settings-updated", result.clone());
            Ok(result)
        }
        "list_bookmarks" | "list_replacement_rules" | "reading_history_resource" => {
            let resource = match request.command.as_str() {
                "list_bookmarks" => {
                    legado_lib::reading_tools::bookmarks_resource(state.app.resource_store()).await
                }
                "list_replacement_rules" => {
                    legado_lib::reading_tools::replacement_rules_resource(
                        state.app.resource_store(),
                    )
                    .await
                }
                _ => {
                    legado_lib::reading_tools::reading_history_resource(state.app.resource_store())
                        .await
                }
            }
            .map_err(internal_error)?;
            Ok(state.app.resource_descriptor(&resource))
        }
        "upsert_bookmark"
        | "delete_bookmark"
        | "record_reading_session"
        | "delete_reading_history_for_book"
        | "clear_reading_history"
        | "upsert_replacement_rule"
        | "delete_replacement_rule"
        | "set_book_groups"
        | "create_shelf_group"
        | "rename_shelf_group"
        | "delete_shelf_group"
        | "set_shelf_sort"
        | "set_shelf_order" => {
            let _operation = state.app.operation_read().await;
            let (kind, resource) = match request.command.as_str() {
                "upsert_bookmark" => (
                    "bookmarks",
                    legado_lib::reading_tools::upsert_bookmark(
                        state.app.resource_store(),
                        typed_arg(&args, "input")?,
                    )
                    .await,
                ),
                "delete_bookmark" => (
                    "bookmarks",
                    legado_lib::reading_tools::delete_bookmark(
                        state.app.resource_store(),
                        string_arg(&args, "bookmarkId")?,
                    )
                    .await,
                ),
                "record_reading_session" => (
                    "readingHistory",
                    legado_lib::reading_tools::record_reading_session(
                        state.app.resource_store(),
                        typed_arg(&args, "session")?,
                    )
                    .await,
                ),
                "delete_reading_history_for_book" => (
                    "readingHistory",
                    legado_lib::reading_tools::delete_reading_history_for_book(
                        state.app.resource_store(),
                        string_arg(&args, "bookId")?,
                    )
                    .await,
                ),
                "clear_reading_history" => (
                    "readingHistory",
                    legado_lib::reading_tools::clear_reading_history(state.app.resource_store())
                        .await,
                ),
                "upsert_replacement_rule" => (
                    "replacementRules",
                    legado_lib::reading_tools::upsert_replacement_rule(
                        state.app.resource_store(),
                        typed_arg(&args, "input")?,
                    )
                    .await,
                ),
                "delete_replacement_rule" => (
                    "replacementRules",
                    legado_lib::reading_tools::delete_replacement_rule(
                        state.app.resource_store(),
                        string_arg(&args, "ruleId")?,
                    )
                    .await,
                ),
                "set_book_groups" => (
                    "shelf",
                    legado_lib::reading_tools::set_book_groups(
                        state.app.resource_store(),
                        string_arg(&args, "bookId")?,
                        string_array_arg(&args, "groups")?,
                    )
                    .await,
                ),
                "create_shelf_group" => (
                    "shelf",
                    legado_lib::reading_tools::create_shelf_group(
                        state.app.resource_store(),
                        string_arg(&args, "groupName")?,
                    )
                    .await,
                ),
                "rename_shelf_group" => (
                    "shelf",
                    legado_lib::reading_tools::rename_shelf_group(
                        state.app.resource_store(),
                        string_arg(&args, "oldName")?,
                        string_arg(&args, "newName")?,
                    )
                    .await,
                ),
                "delete_shelf_group" => (
                    "shelf",
                    legado_lib::reading_tools::delete_shelf_group(
                        state.app.resource_store(),
                        string_arg(&args, "groupName")?,
                    )
                    .await,
                ),
                "set_shelf_sort" => (
                    "shelf",
                    legado_lib::reading_tools::set_shelf_sort(
                        state.app.resource_store(),
                        typed_arg(&args, "key")?,
                        typed_arg(&args, "order")?,
                    )
                    .await,
                ),
                _ => (
                    "shelf",
                    legado_lib::reading_tools::set_shelf_order(
                        state.app.resource_store(),
                        string_array_arg(&args, "orderedBookIds")?,
                    )
                    .await,
                ),
            };
            let resource = resource.map_err(internal_error)?;
            let descriptor = state.app.resource_descriptor(&resource);
            if kind == "shelf" {
                state.emit("shelf-updated", descriptor.clone());
            } else {
                state.emit(
                    "resource-updated",
                    json!({ "kind": kind, "resource": descriptor }),
                );
            }
            Ok(descriptor)
        }
        "list_discovery_categories" | "list_rss_categories" => {
            let source_id = string_arg(&args, "sourceId")?;
            let result = if request.command == "list_discovery_categories" {
                legado_lib::discovery::list_categories(&state.app, source_id, Some(false)).await
            } else {
                legado_lib::rss::list_rss_categories(&state.app, source_id).await
            }
            .map_err(internal_error)?;
            let kind = if request.command == "list_discovery_categories" {
                "discoveryCategories"
            } else {
                "rssCategories"
            };
            state.emit(
                "resource-updated",
                json!({ "kind": kind, "resource": result["resource"] }),
            );
            Ok(result)
        }
        "list_discovery_books" | "list_rss_articles" => {
            let source_id = string_arg(&args, "sourceId")?;
            let category_id = string_arg(&args, "categoryId")?;
            let page = args.get("page").and_then(Value::as_u64).unwrap_or(1) as u32;
            let result = if request.command == "list_discovery_books" {
                legado_lib::discovery::list_books(
                    &state.app,
                    source_id,
                    category_id,
                    page,
                    Some(false),
                )
                .await
            } else {
                legado_lib::rss::list_rss_articles(&state.app, source_id, category_id, page).await
            }
            .map_err(internal_error)?;
            let kind = if request.command == "list_discovery_books" {
                "discoveryResults"
            } else {
                "rssArticles"
            };
            state.emit(
                "resource-updated",
                json!({ "kind": kind, "resource": result["resource"] }),
            );
            Ok(result)
        }
        "open_rss_article" => {
            let result = legado_lib::rss::read_standard_feed_article(
                &state.app,
                string_arg(&args, "sourceId")?,
                string_arg(&args, "articleId")?,
            )
            .await
            .map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "rssArticle", "resource": result["resource"] }),
            );
            Ok(result)
        }
        "list_discovery_favorites" => {
            let _operation = state.app.operation_read().await;
            let resource = legado_lib::discovery::favorites_resource(state.app.resource_store())
                .await
                .map_err(internal_error)?;
            Ok(json!({
                "resource": state.app.resource_descriptor(&resource),
            }))
        }
        "set_discovery_favorite" => {
            let _operation = state.app.operation_read().await;
            let resource = legado_lib::discovery::set_favorite(
                &state.app,
                string_arg(&args, "sourceId")?,
                string_arg(&args, "categoryId")?,
                args.get("favorite")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
            .await
            .map_err(internal_error)?;
            let descriptor = state.app.resource_descriptor(&resource);
            state.emit(
                "resource-updated",
                json!({ "kind": "discoveryFavorites", "resource": descriptor }),
            );
            Ok(descriptor)
        }
        "get_home_config" => state.app.get_home_config().await.map_err(internal_error),
        "save_home_config" => {
            let config = typed_arg(&args, "config")?;
            let resource = state
                .app
                .save_home_config(config)
                .await
                .map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "homeConfig", "resource": resource }),
            );
            Ok(resource)
        }
        "get_txt_toc_rules" => state.app.get_txt_toc_rules().await.map_err(internal_error),
        "upsert_txt_toc_rule" => {
            let rule = typed_arg(&args, "rule")?;
            let resource = state.app.upsert_txt_toc_rule(rule).await.map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "txtTocRules", "resource": resource }),
            );
            Ok(resource)
        }
        "delete_txt_toc_rule" => {
            let resource = state
                .app
                .delete_txt_toc_rule(string_arg(&args, "ruleId")?)
                .await
                .map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "txtTocRules", "resource": resource }),
            );
            Ok(resource)
        }
        "get_rss_state" => state.app.get_rss_state().await.map_err(internal_error),
        "set_rss_filter" => {
            let source_id = string_arg(&args, "sourceId")?;
            let filter = string_arg(&args, "filter")?;
            let resource = state
                .app
                .set_rss_filter(source_id, filter)
                .await
                .map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "rssState", "resource": resource }),
            );
            Ok(resource)
        }
        "set_rss_article_state" => {
            let source_id = string_arg(&args, "sourceId")?;
            let article_id = string_arg(&args, "articleId")?;
            let is_read = optional_bool_arg(&args, "isRead")?;
            let is_favorite = optional_bool_arg(&args, "isFavorite")?;
            let resource = state
                .app
                .set_rss_article_state(source_id, article_id, is_read, is_favorite)
                .await
                .map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "rssState", "resource": resource }),
            );
            Ok(resource)
        }
        "unsubscribe_rss" => {
            let source_id = string_arg(&args, "sourceId")?;
            let result = state
                .app
                .unsubscribe_rss(source_id)
                .await
                .map_err(internal_error)?;
            state.emit(
                "resource-updated",
                json!({ "kind": "rssState", "resource": result["resource"] }),
            );
            state.emit("sources-updated", result["sources"].clone());
            Ok(result)
        }
        "create_backup_from_picker" => {
            let Some(path) = selected_path("LEGADO_BROWSER_HARNESS_BACKUP_OUTPUT") else {
                return Ok(Json(json!({ "ok": true, "value": { "cancelled": true } })));
            };
            state
                .app
                .create_backup(&path)
                .await
                .map_err(internal_error)?;
            Ok(json!({ "backup": true }))
        }
        "restore_backup_from_picker" => {
            let Some(path) = selected_path("LEGADO_BROWSER_HARNESS_BACKUP_INPUT") else {
                return Ok(Json(json!({ "ok": true, "value": { "cancelled": true } })));
            };
            let bootstrap = state
                .app
                .restore_backup(&path)
                .await
                .map_err(internal_error)?;
            state.emit("app-state-updated", bootstrap.clone());
            state.emit("shelf-updated", bootstrap["shelf"].clone());
            state.emit("settings-updated", bootstrap["settings"].clone());
            state.emit("sources-updated", bootstrap["sources"].clone());
            Ok(json!({ "restored": true, "bootstrap": bootstrap }))
        }
        other => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Unknown app command: {other}") })),
            ));
        }
    }?;
    Ok(Json(json!({ "ok": true, "value": value })))
}

async fn events(
    State(state): State<HarnessState>,
    Query(query): Query<EventsQuery>,
) -> Json<Value> {
    let events = state
        .events
        .records
        .lock()
        .expect("browser harness event lock");
    Json(json!({
        "events": events.iter().filter(|event| event.id > query.after).collect::<Vec<_>>(),
    }))
}

fn string_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, (StatusCode, Json<Value>)> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Missing string argument '{key}'") })),
        )
    })
}

fn optional_bool_arg(args: &Value, key: &str) -> Result<Option<bool>, (StatusCode, Json<Value>)> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Argument '{key}' must be a boolean") })),
        )),
    }
}

fn string_array_arg(args: &Value, key: &str) -> Result<Vec<String>, (StatusCode, Json<Value>)> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Missing string array argument '{key}'") })),
            )
        })
}

fn usize_arg(args: &Value, key: &str) -> Result<usize, (StatusCode, Json<Value>)> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Missing non-negative integer argument '{key}'") })),
            )
        })
}

fn typed_arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, (StatusCode, Json<Value>)> {
    let value = args.get(key).cloned().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Missing argument '{key}'") })),
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid argument '{key}': {error}") })),
        )
    })
}

fn selected_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable).map(PathBuf::from)
}

fn internal_error(message: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": message })),
    )
}

async fn browser_cors(request: Request<Body>, next: Next) -> Response<Body> {
    let origin = request
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value == "http://127.0.0.1:1420" || *value == "http://localhost:1420")
        .map(str::to_owned);
    let is_options = request.method() == Method::OPTIONS;
    let mut response = if is_options {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    if let Some(origin) = origin {
        if let Ok(value) = HeaderValue::from_str(&origin) {
            response
                .headers_mut()
                .insert("access-control-allow-origin", value);
        }
        if is_options {
            response.headers_mut().insert(
                "access-control-allow-methods",
                HeaderValue::from_static("GET, POST, OPTIONS"),
            );
            response.headers_mut().insert(
                "access-control-allow-headers",
                HeaderValue::from_static("content-type"),
            );
            response
                .headers_mut()
                .insert("access-control-max-age", HeaderValue::from_static("600"));
        }
        response
            .headers_mut()
            .insert("vary", HeaderValue::from_static("Origin"));
    }
    response
}
