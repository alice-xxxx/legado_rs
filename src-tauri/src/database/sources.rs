//! 书源行的派生字段和关联缓存，由公共写入层统一维护。
use super::*;
use rusqlite::{params_from_iter, types::Value as SqlValue};

pub(super) fn values(
    db: &Connection,
    values: Map<String, Value>,
) -> Result<Map<String, Value>, ResourceError> {
    let source = match values.get("definition_json").unwrap_or(&Value::Null) {
        Value::String(text) => {
            serde_json::from_str(text).map_err(|error| ResourceError::new(error.to_string()))?
        }
        value => value.clone(),
    };
    let (name, group, enabled, url) =
        crate::application::validate_source_definition(&source).map_err(ResourceError::new)?;
    let matching: Option<(String, i64)> = db.query_row(
        "SELECT id,position FROM sources WHERE coalesce(json_extract(definition_json,'$.bookSourceUrl'),json_extract(definition_json,'$.url'),json_extract(definition_json,'$.sourceUrl'))=?1 ORDER BY position LIMIT 1",
        [&url], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    let id = values
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| matching.as_ref().map(|(id, _)| id.clone()))
        .unwrap_or_else(|| format!("source-{:016x}", crate::application::stable_hash(&url)));
    if matching.as_ref().is_some_and(|(other, _)| other != &id) {
        return Err(ResourceError::new(
            "Another imported source already uses that bookSourceUrl",
        ));
    }
    let record = crate::application::SourceRecord {
        id: id.clone(),
        name,
        group,
        enabled,
        source,
    };
    crate::application::validate_source_records(std::slice::from_ref(&record))
        .map_err(ResourceError::new)?;
    let position: Option<i64> = db
        .query_row("SELECT position FROM sources WHERE id=?1", [&id], |row| {
            row.get(0)
        })
        .optional()?;
    let position = match position {
        Some(position) => position,
        None => db.query_row(
            "SELECT coalesce(max(position)+1,0) FROM sources",
            [],
            |row| row.get(0),
        )?,
    };
    Ok(super::collections::source_values(
        position as usize,
        &record,
    ))
}

pub(super) fn delete_related(
    db: &Connection,
    condition: &str,
    params: &[SqlValue],
) -> Result<(), ResourceError> {
    let mut statement = db.prepare(&format!("SELECT id,is_rss FROM sources{condition}"))?;
    let sources = statement
        .query_map(params_from_iter(params), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, rss) in sources {
        for table in [
            "rss_subscriptions",
            "rss_articles",
            "discovery_sources",
            "discovery_favorites",
        ] {
            db.execute(&format!("DELETE FROM {table} WHERE source_id=?1"), [&id])?;
        }
        if rss {
            let book_id = format!("rss-{id}");
            db.execute("DELETE FROM books WHERE id=?1 AND kind='html'", [&book_id])?;
            // 文件删除意图和数据一起提交，清理失败后可重试。
            db.execute(
                "INSERT OR IGNORE INTO pending_cleanup(path) VALUES(?1)",
                [format!("cache/html/{book_id}")],
            )?;
        }
    }
    Ok(())
}
