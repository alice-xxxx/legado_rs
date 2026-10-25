//! 书源和朗读配置：固定列写入，关系行批量读取。
use super::*;
use tauri_plugin_source_engine::TextOr;
use std::collections::{BTreeMap, HashMap};

fn script<T>(value: &Option<TextOr<T>>) -> Option<&str> {
    match value {
        Some(TextOr::Text(text)) => Some(text),
        _ => None,
    }
}
impl Database {
    pub(crate) fn sources(&self) -> Result<Vec<Source>> {
        db::read(self, |s| {
            let mut sources = s.read("SELECT s.*,o.* FROM sources s LEFT JOIN source_options o ON o.owner_id=s.id WHERE s.removed=0 ORDER BY s.position", [], |r| Ok(Source {
                id: r.get("id")?,
                definition: SourceDefinition {
                    book_source_name: r.get("name")?,
                    book_source_url: r.get("url")?,
                    book_source_group: r.get("source_group")?,
                    enabled: r.get("enabled")?,
                    book_source_type: r.get("book_source_type")?,
                    book_url_pattern: r.get("book_url_pattern")?,
                    enabled_explore: r.get("enabled_explore")?,
                    enabled_review: r.get("enabled_review")?,
                    js_lib: r.get("js_lib")?,
                    enabled_cookie_jar: r.get("enabled_cookie_jar")?,
                    enable_dangerous_api: r.get("enable_dangerous_api")?,
                    concurrent_rate: r.get("concurrent_rate")?,
                    login_url: r.get("login_url")?,
                    login_check_js: r.get("login_check_js")?,
                    cover_decode_js: r.get("cover_decode_js")?,
                    book_source_comment: r.get("book_source_comment")?,
                    variable_comment: r.get("variable_comment")?,
                    last_update_time: r.get("last_update_time")?,
                    respond_time: r.get("respond_time")?,
                    weight: r.get("weight")?,
                    explore_screen: r.get("explore_screen")?,
                    explore_style: r.get("explore_style")?,
                    search_url: r.get("search_url")?,
                    legado_rs_user_agent_override: r.get("user_agent_override")?,
                    header: r.get::<_,Option<String>>("header_script")?.map(TextOr::Text),
                    login_ui: r.get::<_,Option<String>>("login_ui_script")?.map(TextOr::Text),
                    explore_url: r.get::<_,Option<String>>("explore_url")?.map(TextOr::Text),
                    ..Default::default()
                },
            }))?;
            let positions: HashMap<_, _> = sources
                .iter()
                .enumerate()
                .map(|(index, source)| (source.id.clone(), index))
                .collect();
            for (owner,stage,key,expression) in s.read("SELECT r.source_id,r.stage,r.target,r.expression FROM source_rules r JOIN sources s ON s.id=r.source_id WHERE s.removed=0", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))? {
                let source = &mut sources[positions[&owner]].definition;
                let rules = match stage.as_str() {
                    "ruleSearch" => &mut source.rule_search,
                    "ruleExplore" => &mut source.rule_explore,
                    "ruleBookInfo" => &mut source.rule_book_info,
                    "ruleToc" => &mut source.rule_toc,
                    "ruleContent" => &mut source.rule_content,
                    "ruleReview" => &mut source.rule_review,
                    _ => continue,
                };
                rules.get_or_insert_with(BTreeMap::new).insert(key,expression);
            }
            for (owner,name,value) in s.read("SELECT h.owner_id,h.name,h.value FROM source_headers h JOIN sources s ON s.id=h.owner_id WHERE s.removed=0 ORDER BY h.owner_id,h.position", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))? {
                if let TextOr::Data(headers) = sources[positions[&owner]].definition.header.get_or_insert_with(|| TextOr::Data(BTreeMap::new())) { headers.insert(name,value); }
            }
            for (owner,item) in s.read("SELECT i.* FROM source_login_items i JOIN sources s ON s.id=i.owner_id WHERE s.removed=0 ORDER BY i.owner_id,i.position", [], |r| Ok((r.get::<_,String>("owner_id")?,LoginItem {
                name:r.get("name")?,r#type:r.get("type")?,action:r.get("action")?,cols:r.get("cols")?,rows:r.get("rows")?,width:r.get("width")?,options:Vec::new(),
            })))? {
                if let TextOr::Data(items) = sources[positions[&owner]].definition.login_ui.get_or_insert_with(|| TextOr::Data(Vec::new())) { items.push(item); }
            }
            for (owner,position,value) in s.read("SELECT c.owner_id,c.item_position,c.value FROM source_login_choices c JOIN sources s ON s.id=c.owner_id WHERE s.removed=0 ORDER BY c.owner_id,c.item_position,c.position", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,usize>(1)?,r.get::<_,String>(2)?)))? {
                if let Some(TextOr::Data(items)) = &mut sources[positions[&owner]].definition.login_ui { items[position].options.push(value); }
            }
            for (owner,link) in s.read("SELECT l.* FROM source_explore_links l JOIN sources s ON s.id=l.source_id WHERE s.removed=0 ORDER BY l.source_id,l.position", [], |r| Ok((r.get::<_,String>("source_id")?,ExploreLink { title:r.get("title")?,url:r.get("url")?,kind:r.get("kind")? })))? {
                if let TextOr::Data(links) = sources[positions[&owner]].definition.explore_url.get_or_insert_with(|| TextOr::Data(Vec::new())) { links.push(link); }
            }
            Ok(sources)
        })
    }
}

pub(super) fn save_source(s: &Session<'_>, input: SourceInput) -> Result<String> {
    let (group_set, group) = input.group.parts();
    let Some(d) = input.definition else {
        let id = input
            .id
            .ok_or_else(|| ResourceError::new("修改书源需要 ID"))?;
        if s.write("UPDATE sources SET enabled=coalesce(?2,enabled),source_group=CASE WHEN ?3 THEN ?4 ELSE source_group END,removed=coalesce(?5,removed) WHERE id=?1", params![id,input.enabled,group_set,group,input.removed])? == 0 {
            return Err(ResourceError::new("书源不存在"));
        }
        return Ok(id);
    };
    let id = input
        .id
        .or(s
            .read(
                "SELECT id FROM sources WHERE url=?1",
                [&d.book_source_url],
                |r| r.get(0),
            )?
            .pop())
        .unwrap_or_else(|| {
            format!(
                "source-{:016x}",
                crate::support::stable_hash(&d.book_source_url)
            )
        });
    let kind = match d.book_source_type.unwrap_or(0) {
        1 => "audio",
        2 => "image",
        4 => "video",
        5 => "html",
        _ => "text",
    };
    s.write("INSERT INTO sources(id,position,url,name,source_group,enabled,removed,is_rss,content_type,user_agent_override)
        VALUES(?1,coalesce((SELECT position FROM sources WHERE id=?1),(SELECT coalesce(max(position),-1)+1 FROM sources)),?2,?3,?4,?5,?6,?7,?8,?9)
        ON CONFLICT(id) DO UPDATE SET url=excluded.url,name=excluded.name,source_group=excluded.source_group,enabled=excluded.enabled,
        removed=excluded.removed,is_rss=excluded.is_rss,content_type=excluded.content_type,user_agent_override=excluded.user_agent_override",
        params![id,d.book_source_url,d.book_source_name,if group_set {group.as_ref()} else {d.book_source_group.as_ref()},input.enabled.or(d.enabled).unwrap_or(true),input.removed.unwrap_or(false),d.book_source_type==Some(5),kind,d.legado_rs_user_agent_override])?;
    s.write("DELETE FROM source_options WHERE owner_id=?1", [&id])?;
    s.write("INSERT INTO source_options(owner_id,book_source_type,book_url_pattern,enabled_explore,enabled_review,js_lib,enabled_cookie_jar,enable_dangerous_api,concurrent_rate,login_url,login_check_js,cover_decode_js,book_source_comment,variable_comment,last_update_time,respond_time,weight,explore_screen,explore_style,search_url,explore_url,header_script,login_ui_script) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![id,d.book_source_type,d.book_url_pattern,d.enabled_explore,d.enabled_review,d.js_lib,d.enabled_cookie_jar,d.enable_dangerous_api,d.concurrent_rate,d.login_url,d.login_check_js,d.cover_decode_js,d.book_source_comment,d.variable_comment,d.last_update_time,d.respond_time,d.weight,d.explore_screen,d.explore_style,d.search_url,script(&d.explore_url),script(&d.header),script(&d.login_ui)])?;
    s.write("DELETE FROM source_headers WHERE owner_id=?1", [&id])?;
    s.write("DELETE FROM source_login_items WHERE owner_id=?1", [&id])?;
    s.write("DELETE FROM source_explore_links WHERE source_id=?1", [&id])?;
    s.write("DELETE FROM source_rules WHERE source_id=?1", [&id])?;
    if let Some(TextOr::Data(headers)) = &d.header {
        for (position, (name, value)) in headers.iter().enumerate() {
            s.write(
                "INSERT INTO source_headers(owner_id,position,name,value) VALUES(?1,?2,?3,?4)",
                params![id, position as i64, name, value],
            )?;
        }
    }
    if let Some(TextOr::Data(items)) = &d.login_ui {
        for (position, item) in items.iter().enumerate() {
            s.write("INSERT INTO source_login_items(owner_id,position,name,type,action,cols,rows,width) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id,position as i64,item.name,item.r#type,item.action,item.cols,item.rows,item.width])?;
            for (choice, value) in item.options.iter().enumerate() {
                s.write("INSERT INTO source_login_choices(owner_id,item_position,position,value) VALUES(?1,?2,?3,?4)",params![id,position as i64,choice as i64,value])?;
            }
        }
    }
    if let Some(TextOr::Data(links)) = &d.explore_url {
        for (position, link) in links.iter().enumerate() {
            s.write("INSERT INTO source_explore_links(source_id,position,title,url,kind) VALUES(?1,?2,?3,?4,?5)",params![id,position as i64,link.title,link.url,link.kind])?;
        }
    }
    for (stage, rules) in [
        ("ruleSearch", &d.rule_search),
        ("ruleExplore", &d.rule_explore),
        ("ruleBookInfo", &d.rule_book_info),
        ("ruleToc", &d.rule_toc),
        ("ruleContent", &d.rule_content),
        ("ruleReview", &d.rule_review),
    ] {
        if let Some(rules) = rules {
            for (target, expression) in rules {
                s.write("INSERT INTO source_rules(source_id,stage,target,expression) VALUES(?1,?2,?3,?4)",params![id,stage,target,expression])?;
            }
        }
    }
    Ok(id)
}

pub(super) fn save_tts(s: &Session<'_>, input: TtsInput) -> Result<()> {
    let d = input.definition;
    s.write("INSERT INTO http_tts_configs(id,position,name,enabled) VALUES(?1,coalesce((SELECT position FROM http_tts_configs WHERE id=?1),(SELECT coalesce(max(position),-1)+1 FROM http_tts_configs)),?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,enabled=excluded.enabled",params![input.id,input.name,input.enabled])?;
    s.write(
        "DELETE FROM http_tts_options WHERE owner_id=?1",
        [&input.id],
    )?;
    s.write("INSERT INTO http_tts_options(owner_id,id,name,url,content_type,concurrent_rate,login_url,login_check_js,js_lib,enabled_cookie_jar,enable_dangerous_api,last_update_time,header_script,login_ui_script) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![input.id,d.id,d.name,d.url,d.content_type,d.concurrent_rate,d.login_url,d.login_check_js,d.js_lib,d.enabled_cookie_jar,d.enable_dangerous_api,d.last_update_time,script(&d.header),script(&d.login_ui)])?;
    s.write(
        "DELETE FROM http_tts_headers WHERE owner_id=?1",
        [&input.id],
    )?;
    s.write(
        "DELETE FROM http_tts_login_items WHERE owner_id=?1",
        [&input.id],
    )?;
    if let Some(TextOr::Data(headers)) = d.header {
        for (position, (name, value)) in headers.into_iter().enumerate() {
            s.write(
                "INSERT INTO http_tts_headers(owner_id,position,name,value) VALUES(?1,?2,?3,?4)",
                params![input.id, position as i64, name, value],
            )?;
        }
    }
    if let Some(TextOr::Data(items)) = d.login_ui {
        for (position, item) in items.into_iter().enumerate() {
            s.write("INSERT INTO http_tts_login_items(owner_id,position,name,type,action,cols,rows,width) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![input.id,position as i64,item.name,item.r#type,item.action,item.cols,item.rows,item.width])?;
            for (choice, value) in item.options.into_iter().enumerate() {
                s.write("INSERT INTO http_tts_login_choices(owner_id,item_position,position,value) VALUES(?1,?2,?3,?4)",params![input.id,position as i64,choice as i64,value])?;
            }
        }
    }
    Ok(())
}

impl Database {
    pub(crate) fn tts_config(&self, id: &str) -> Result<Option<TtsDefinition>> {
        db::read(self, |s| {
            let Some(mut config) = s.read("SELECT o.* FROM http_tts_options o JOIN http_tts_configs c ON c.id=o.owner_id WHERE c.id=?1 AND c.enabled=1",[id],|r| Ok(TtsDefinition {
                id:r.get("id")?,
                name:r.get("name")?,
                url:r.get("url")?,
                content_type:r.get("content_type")?,
                concurrent_rate:r.get("concurrent_rate")?,
                login_url:r.get("login_url")?,
                login_check_js:r.get("login_check_js")?,
                js_lib:r.get("js_lib")?,
                enabled_cookie_jar:r.get("enabled_cookie_jar")?,
                enable_dangerous_api:r.get("enable_dangerous_api")?,
                last_update_time:r.get("last_update_time")?,
                header:r.get::<_,Option<String>>("header_script")?.map(TextOr::Text),
                login_ui:r.get::<_,Option<String>>("login_ui_script")?.map(TextOr::Text),
            }))?.pop() else { return Ok(None); };
            let headers = s.read(
                "SELECT name,value FROM http_tts_headers WHERE owner_id=?1 ORDER BY position",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?;
            if !headers.is_empty() {
                config.header = Some(TextOr::Data(headers.into_iter().collect()));
            }
            let mut items = s.read("SELECT name,type,action,cols,rows,width FROM http_tts_login_items WHERE owner_id=?1 ORDER BY position",[id],|r|Ok(LoginItem {
                name:r.get(0)?,r#type:r.get(1)?,action:r.get(2)?,cols:r.get(3)?,rows:r.get(4)?,width:r.get(5)?,options:Vec::new(),
            }))?;
            for (index,value) in s.read("SELECT item_position,value FROM http_tts_login_choices WHERE owner_id=?1 ORDER BY item_position,position",[id],|r|Ok((r.get::<_,usize>(0)?,r.get::<_,String>(1)?)))? { items[index].options.push(value); }
            if !items.is_empty() {
                config.login_ui = Some(TextOr::Data(items));
            }
            Ok(Some(config))
        })
    }
}
