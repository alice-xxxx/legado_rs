use super::{
    collections::{list, replace},
    rows::{array, text, Field, Kind::*},
    *,
};
const TASK: &[Field] = &[
    ("id", "id", Text),
    ("position", "_position", Integer),
    ("kind", "kind", Text),
    ("status", "status", Text),
    ("book_id", "bookId", Text),
    ("keyword", "keyword", Text),
    ("source_type", "sourceType", Text),
    ("precision_search", "precisionSearch", Bool),
    ("page", "page", Integer),
    ("from_index", "fromIndex", Integer),
    ("total", "total", Integer),
    ("completed", "completed", Integer),
    ("search_id", "searchId", Text),
    ("result_json", "result", Json),
    ("error", "error", Text),
    ("created_at_ms", "createdAtMs", Integer),
    ("updated_at_ms", "updatedAtMs", Integer),
];
pub(super) fn read(db: &Connection) -> Result<Value, ResourceError> {
    let mut tasks = list(db, "tasks", TASK)?;
    for (table, column, name) in [
        ("task_chapters", "chapter_id", "chapterIds"),
        ("task_sources", "source_id", "sourceIds"),
    ] {
        let mut stmt = db.prepare(&format!(
            "SELECT task_id,{column} FROM {table} ORDER BY task_id,position"
        ))?;
        let mut by_task = HashMap::<String, Vec<String>>::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, value) = row?;
            by_task.entry(id).or_default().push(value);
        }
        for task in &mut tasks {
            if let Some(ids) = by_task.remove(task["id"].as_str().unwrap_or("")) {
                task[name] = json!(ids);
            }
        }
    }
    Ok(json!({"schemaVersion":1,"tasks":tasks}))
}
pub(super) fn write(db: &Connection, value: &Value) -> Result<(), ResourceError> {
    replace(db, "tasks", TASK, &["id"], array(value, "tasks"))?;
    for task in array(value, "tasks") {
        let id = text(task, "id")?;
        for (table, column, name) in [
            ("task_chapters", "chapter_id", "chapterIds"),
            ("task_sources", "source_id", "sourceIds"),
        ] {
            for (position, item) in array(task, name).iter().enumerate() {
                db.execute(
                    &format!("INSERT INTO {table}(task_id,position,{column}) VALUES(?1,?2,?3)"),
                    params![id, position as i64, item.as_str()],
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn insert(db: &Connection, task: &Value) -> Result<(), ResourceError> {
    let mut row = task.clone();
    row["_position"] = json!(db.query_row(
        "SELECT coalesce(max(position)+1,0) FROM tasks",
        [],
        |r| r.get::<_, i64>(0)
    )?);
    rows::upsert(db, "tasks", TASK, &["id"], &row)?;
    let id = text(task, "id")?;
    for (table, column, name) in [
        ("task_chapters", "chapter_id", "chapterIds"),
        ("task_sources", "source_id", "sourceIds"),
    ] {
        for (position, item) in array(task, name).iter().enumerate() {
            db.execute(
                &format!("INSERT INTO {table}(task_id,position,{column}) VALUES(?1,?2,?3)"),
                params![id, position as i64, item.as_str()],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn update_states(db: &Connection, states: &[Value]) -> Result<(), ResourceError> {
    let mut update = db.prepare_cached("UPDATE tasks SET status=?2,completed=?3,total=?4,error=?5,result_json=?6,updated_at_ms=?7 WHERE id=?1")?;
    for state in states {
        let result = state
            .get("result")
            .filter(|v| !v.is_null())
            .map(Value::to_string);
        if update.execute(params![
            text(state, "id")?,
            text(state, "status")?,
            state["completed"].as_i64(),
            state["total"].as_i64(),
            state["error"].as_str(),
            result,
            state["updatedAtMs"].as_i64()
        ])? != 1
        {
            return Err(ResourceError::new("Task no longer exists"));
        }
    }
    Ok(())
}
