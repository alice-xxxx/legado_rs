//! Fixed, domain-specific row mappings at the JSON API boundary.
use crate::resources::ResourceError;
use rusqlite::{
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
    Connection, Params,
};
use serde_json::{Map, Value};

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Text,
    Integer,
    Real,
    Bool,
    Json,
}
pub(super) type Field = (&'static str, &'static str, Kind);

pub(super) fn select<P: Params>(
    db: &Connection,
    table: &str,
    fields: &[Field],
    suffix: &str,
    params: P,
) -> Result<Vec<Value>, ResourceError> {
    let sql = format!(
        "SELECT {} FROM {table} {suffix}",
        fields.iter().map(|f| f.0).collect::<Vec<_>>().join(",")
    );
    let mut statement = db.prepare(&sql)?;
    let mut cursor = statement.query(params)?;
    let mut result = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut item = Map::new();
        for (index, (_, name, kind)) in fields.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => continue,
                ValueRef::Integer(v) => {
                    if matches!(kind, Kind::Bool) {
                        Value::Bool(v != 0)
                    } else {
                        v.into()
                    }
                }
                ValueRef::Real(v) => serde_json::json!(v),
                ValueRef::Text(v) if matches!(kind, Kind::Json) => {
                    serde_json::from_slice(v).map_err(|e| ResourceError::new(e.to_string()))?
                }
                ValueRef::Text(v) => String::from_utf8_lossy(v).into_owned().into(),
                ValueRef::Blob(_) => {
                    return Err(ResourceError::new("Unexpected binary business field"))
                }
            };
            item.insert((*name).into(), value);
        }
        result.push(Value::Object(item));
    }
    Ok(result)
}

pub(super) fn upsert(
    db: &Connection,
    table: &str,
    fields: &[Field],
    keys: &[&str],
    value: &Value,
) -> Result<(), ResourceError> {
    let columns = fields.iter().map(|f| f.0).collect::<Vec<_>>();
    let updates = columns
        .iter()
        .filter(|column| !keys.contains(column))
        .map(|c| format!("{c}=excluded.{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "INSERT INTO {table}({}) VALUES({}) ON CONFLICT({}) DO UPDATE SET {updates}",
        columns.join(","),
        vec!["?"; fields.len()].join(","),
        keys.join(",")
    );
    let values = fields
        .iter()
        .map(|(_, name, kind)| {
            let v = &value[*name];
            if v.is_null() {
                return SqlValue::Null;
            }
            match kind {
                Kind::Json => SqlValue::Text(v.to_string()),
                Kind::Bool => SqlValue::Integer(i64::from(v.as_bool().unwrap_or(false))),
                Kind::Integer => SqlValue::Integer(
                    v.as_i64()
                        .or_else(|| v.as_u64().map(|n| n.min(i64::MAX as u64) as i64))
                        .unwrap_or(0),
                ),
                Kind::Real => SqlValue::Real(v.as_f64().unwrap_or(0.0)),
                Kind::Text => SqlValue::Text(
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string()),
                ),
            }
        })
        .collect::<Vec<_>>();
    db.execute(&sql, params_from_iter(values))?;
    Ok(())
}

pub(super) fn array<'a>(value: &'a Value, name: &str) -> &'a [Value] {
    value[name]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}
pub(super) fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, ResourceError> {
    value[name]
        .as_str()
        .ok_or_else(|| ResourceError::new(format!("Missing {name}")))
}
