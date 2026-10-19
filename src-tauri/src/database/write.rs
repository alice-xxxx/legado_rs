//! Rust 和前端共用的参数化写入；一批操作在同一个事务内提交。
use super::{Database, ResourceError};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection};
use serde::Deserialize;
use serde_json::{json, Map, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Change {
    // 阅读记录会同步累计统计，保证去重和统计更新处于同一事务。
    RecordSession {
        session: crate::reading_tools::ReadingSession,
    },
    Insert {
        table: String,
        values: Map<String, Value>,
    },
    Upsert {
        table: String,
        values: Map<String, Value>,
        keys: Vec<String>,
    },
    Update {
        table: String,
        values: Map<String, Value>,
        filter: Map<String, Value>,
    },
    Delete {
        table: String,
        filter: Map<String, Value>,
    },
}

// 标识符只接受普通表名和列名，值始终通过占位符绑定。
fn identifier(name: &str) -> Result<String, ResourceError> {
    if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
        return Err(ResourceError::new("Invalid database identifier"));
    }
    Ok(format!("\"{name}\""))
}

pub(super) fn sql_value(value: &Value) -> Result<SqlValue, ResourceError> {
    Ok(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
        Value::Number(v) => {
            if let Some(v) = v.as_i64() {
                SqlValue::Integer(v)
            } else if v.is_u64() {
                return Err(ResourceError::new("Database integer is out of range"));
            } else {
                SqlValue::Real(
                    v.as_f64()
                        .ok_or_else(|| ResourceError::new("Invalid database number"))?,
                )
            }
        }
        Value::String(v) => SqlValue::Text(v.clone()),
        // JSON 字段统一序列化，调用方无需自行转字符串。
        v => SqlValue::Text(v.to_string()),
    })
}

fn conditions(
    filter: &Map<String, Value>,
    params: &mut Vec<SqlValue>,
) -> Result<String, ResourceError> {
    let mut conditions = Vec::new();
    for (column, value) in filter {
        let column = identifier(column)?;
        if let Value::Array(values) = value {
            for value in values {
                params.push(sql_value(value)?);
            }
            conditions.push(format!(
                "{column} IN ({})",
                vec!["?"; values.len()].join(",")
            ));
        } else {
            params.push(sql_value(value)?);
            conditions.push(format!("{column} IS ?"));
        }
    }
    Ok(if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    })
}

// 行映射和通用写接口复用同一段 SQL 生成逻辑。
pub(super) fn insert(
    db: &Connection,
    table: &str,
    values: Vec<(String, SqlValue)>,
    keys: Option<&[String]>,
) -> Result<(), ResourceError> {
    if values.is_empty() {
        return Err(ResourceError::new("Database values are empty"));
    }
    let table = identifier(table)?;
    let columns = values
        .iter()
        .map(|(name, _)| identifier(name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut sql = format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        columns.join(","),
        vec!["?"; values.len()].join(",")
    );
    if let Some(keys) = keys {
        if keys.is_empty()
            || keys
                .iter()
                .any(|key| !values.iter().any(|(name, _)| name == key))
        {
            return Err(ResourceError::new("Upsert keys must name supplied values"));
        }
        let updates = values
            .iter()
            .filter(|(name, _)| !keys.contains(name))
            .map(|(name, _)| identifier(name).map(|name| format!("{name}=excluded.{name}")))
            .collect::<Result<Vec<_>, _>>()?;
        let keys = keys
            .iter()
            .map(|key| identifier(key))
            .collect::<Result<Vec<_>, _>>()?;
        sql.push_str(&format!(
            " ON CONFLICT ({}) DO {}",
            keys.join(","),
            if updates.is_empty() {
                "NOTHING".into()
            } else {
                format!("UPDATE SET {}", updates.join(","))
            }
        ));
    }
    db.execute(
        &sql,
        params_from_iter(values.into_iter().map(|(_, value)| value)),
    )?;
    Ok(())
}

pub(super) fn apply(db: &Connection, mut change: Change) -> Result<(), ResourceError> {
    if let Change::Insert { table, values } | Change::Upsert { table, values, .. } = &mut change {
        if table == "sources" && !values.contains_key("content_type") {
            *values = super::sources::values(db, std::mem::take(values))?;
        }
    }
    if let Change::Update {
        table,
        values,
        filter,
    } = &mut change
    {
        if table == "sources" && values.contains_key("definition_json") {
            let id = filter.get("id").and_then(Value::as_str).ok_or_else(|| {
                ResourceError::new("Source definition update requires one source ID")
            })?;
            values.insert("id".into(), json!(id));
            *values = super::sources::values(db, std::mem::take(values))?;
        }
    }
    match change {
        Change::RecordSession { session } => {
            if session.session_id.is_empty()
                || session.book_id.is_empty()
                || session.ended_at_ms == 0
                || session.duration_ms == 0
                || session.duration_ms > session.ended_at_ms
                || session.ended_at_ms > i64::MAX as u64
                || !(-840..=840).contains(&session.utc_offset_minutes)
            {
                return Err(ResourceError::new("Invalid reading session"));
            }
            super::history::record_session(db, &session)
        }
        Change::Insert { table, values } => insert(
            db,
            &table,
            values
                .into_iter()
                .map(|(key, value)| Ok((key, sql_value(&value)?)))
                .collect::<Result<_, ResourceError>>()?,
            None,
        ),
        Change::Upsert {
            table,
            values,
            keys,
        } => insert(
            db,
            &table,
            values
                .into_iter()
                .map(|(key, value)| Ok((key, sql_value(&value)?)))
                .collect::<Result<_, ResourceError>>()?,
            Some(&keys),
        ),
        Change::Update {
            table,
            values,
            filter,
        } => {
            if values.is_empty() || filter.is_empty() {
                return Err(ResourceError::new("Update requires values and a filter"));
            }
            let mut params = Vec::new();
            let mut assignments = Vec::new();
            for (column, value) in values {
                assignments.push(format!("{}=?", identifier(&column)?));
                params.push(sql_value(&value)?);
            }
            let sql = format!(
                "UPDATE {} SET {}{}",
                identifier(&table)?,
                assignments.join(","),
                conditions(&filter, &mut params)?
            );
            db.execute(&sql, params_from_iter(params))?;
            Ok(())
        }
        Change::Delete { table, filter } => {
            let mut params = Vec::new();
            let condition = conditions(&filter, &mut params)?;
            if table == "sources" {
                super::sources::delete_related(db, &condition, &params)?;
            }
            // 空筛选明确表示清空该表；外键级联仍由数据库保证。
            let sql = format!("DELETE FROM {}{}", identifier(&table)?, condition);
            db.execute(&sql, params_from_iter(params))?;
            Ok(())
        }
    }
}

impl Database {
    pub(crate) fn modify(&self, changes: Vec<Change>) -> Result<(), ResourceError> {
        let cleanup = changes
            .iter()
            .any(|change| matches!(change, Change::Delete { table, .. } if table == "sources"));
        self.transaction(move |db| {
            for change in changes {
                apply(db, change)?;
            }
            Ok(())
        })?;
        if cleanup {
            crate::resource_transactions::recover_all(&self.root)
                .map_err(|error| ResourceError::new(error.to_string()))?;
        }
        Ok(())
    }
}
