-- 仅保存普通列和关系；不迁移旧库。
CREATE TABLE sources (
    id TEXT PRIMARY KEY, position INTEGER NOT NULL, url TEXT NOT NULL UNIQUE CHECK(length(url)>0),
    name TEXT NOT NULL CHECK(length(name)>0), source_group TEXT, enabled BOOLEAN NOT NULL DEFAULT 1,
    removed BOOLEAN NOT NULL DEFAULT 0, is_rss BOOLEAN NOT NULL DEFAULT 0,
    content_type TEXT NOT NULL, user_agent_override TEXT
);
CREATE TABLE source_options (
    owner_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
    book_source_type INTEGER, book_url_pattern TEXT, enabled_explore BOOLEAN, enabled_review BOOLEAN,
    js_lib TEXT, enabled_cookie_jar BOOLEAN, enable_dangerous_api BOOLEAN, concurrent_rate TEXT,
    login_url TEXT, login_check_js TEXT, cover_decode_js TEXT, book_source_comment TEXT,
    variable_comment TEXT, last_update_time INTEGER, respond_time INTEGER, weight INTEGER,
    explore_url TEXT, explore_screen TEXT, explore_style INTEGER, search_url TEXT,
    header_script TEXT, login_ui_script TEXT
);
CREATE TABLE source_rules (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE, stage TEXT NOT NULL,
    target TEXT NOT NULL, expression TEXT NOT NULL, PRIMARY KEY(source_id,stage,target)
);
CREATE TABLE source_headers (
    owner_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    name TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(owner_id,position)
);
CREATE TABLE source_login_items (
    owner_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    name TEXT, type TEXT, action TEXT, cols INTEGER, rows INTEGER, width INTEGER,
    PRIMARY KEY(owner_id,position)
);
CREATE TABLE source_login_choices (
    owner_id TEXT NOT NULL, item_position INTEGER NOT NULL, position INTEGER NOT NULL, value TEXT NOT NULL,
    PRIMARY KEY(owner_id,item_position,position),
    FOREIGN KEY(owner_id,item_position) REFERENCES source_login_items(owner_id,position) ON DELETE CASCADE
);
CREATE TABLE source_explore_links (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    title TEXT, url TEXT, kind TEXT, PRIMARY KEY(source_id,position)
);
CREATE TABLE http_tts_configs (
    id TEXT PRIMARY KEY, position INTEGER NOT NULL, name TEXT NOT NULL, enabled BOOLEAN NOT NULL
);
CREATE TABLE http_tts_options (
    owner_id TEXT PRIMARY KEY REFERENCES http_tts_configs(id) ON DELETE CASCADE,
    id INTEGER, name TEXT, url TEXT, content_type TEXT, concurrent_rate TEXT, login_url TEXT,
    login_check_js TEXT, js_lib TEXT, enabled_cookie_jar BOOLEAN, enable_dangerous_api BOOLEAN,
    last_update_time INTEGER, header_script TEXT, login_ui_script TEXT
);
CREATE TABLE http_tts_headers (
    owner_id TEXT NOT NULL REFERENCES http_tts_configs(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    name TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(owner_id,position)
);
CREATE TABLE http_tts_login_items (
    owner_id TEXT NOT NULL REFERENCES http_tts_configs(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    name TEXT, type TEXT, action TEXT, cols INTEGER, rows INTEGER, width INTEGER,
    PRIMARY KEY(owner_id,position)
);
CREATE TABLE http_tts_login_choices (
    owner_id TEXT NOT NULL, item_position INTEGER NOT NULL, position INTEGER NOT NULL, value TEXT NOT NULL,
    PRIMARY KEY(owner_id,item_position,position),
    FOREIGN KEY(owner_id,item_position) REFERENCES http_tts_login_items(owner_id,position) ON DELETE CASCADE
);
CREATE TABLE books (
    id TEXT PRIMARY KEY, content_type TEXT NOT NULL, source_id TEXT REFERENCES sources(id),
    title TEXT NOT NULL DEFAULT 'Untitled', author TEXT, intro TEXT, cover TEXT,
    word_count TEXT, status TEXT, original TEXT, book_url TEXT, page_count INTEGER, url TEXT,
    updated_at_ms INTEGER, information_fetched_at_ms INTEGER, latest_chapter_title TEXT,
    title_override TEXT, author_override TEXT, intro_override TEXT, cover_override TEXT,
    engine_toc_url TEXT, engine_origin TEXT, engine_origin_name TEXT, engine_name TEXT,
    engine_author TEXT, engine_kind TEXT, engine_cover_url TEXT, engine_intro TEXT,
    engine_charset TEXT, engine_type INTEGER, engine_latest_chapter_title TEXT,
    engine_latest_chapter_time INTEGER, engine_word_count TEXT, engine_article_id TEXT
);
CREATE INDEX book_source ON books(source_id);
CREATE TABLE book_tags (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    tag TEXT NOT NULL, PRIMARY KEY(book_id,position)
);
CREATE TABLE book_alternatives (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    alternative_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, PRIMARY KEY(book_id,alternative_id), UNIQUE(book_id,position),
    CHECK(book_id<>alternative_id)
);
CREATE TABLE chapters (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, id TEXT NOT NULL,
    position INTEGER NOT NULL, title TEXT NOT NULL DEFAULT '', volume BOOLEAN NOT NULL DEFAULT 0,
    content_id TEXT REFERENCES chapter_contents(id), engine_url TEXT,
    engine_book_url TEXT, engine_is_vip BOOLEAN, engine_is_pay BOOLEAN, engine_resource_url TEXT,
    engine_tag TEXT, engine_word_count TEXT, engine_start INTEGER, engine_end INTEGER,
    engine_start_fragment_id TEXT, engine_end_fragment_id TEXT,
    PRIMARY KEY(book_id,id), UNIQUE(book_id,position), UNIQUE(book_id,engine_url)
);
CREATE TABLE chapter_contents (
    id TEXT PRIMARY KEY, producer_book_id TEXT, producer_chapter_id TEXT, kind TEXT NOT NULL,
    text_url TEXT, duration_ms INTEGER, protocol TEXT, updated_at_ms INTEGER NOT NULL,
    UNIQUE(producer_book_id,producer_chapter_id),
    FOREIGN KEY(producer_book_id,producer_chapter_id) REFERENCES chapters(book_id,id) ON DELETE SET NULL
);
CREATE INDEX content_consumer ON chapters(content_id);
CREATE TABLE content_pages (
    content_id TEXT NOT NULL REFERENCES chapter_contents(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, id TEXT NOT NULL, url TEXT NOT NULL, width INTEGER, height INTEGER,
    PRIMARY KEY(content_id,position)
);
CREATE TABLE content_streams (
    content_id TEXT NOT NULL REFERENCES chapter_contents(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, label TEXT, url TEXT, protocol TEXT, format TEXT,
    unavailable_reason TEXT, is_default BOOLEAN NOT NULL DEFAULT 0, PRIMARY KEY(content_id,position)
);
CREATE UNIQUE INDEX one_default_stream ON content_streams(content_id) WHERE is_default=1;
CREATE TABLE content_files (
    content_id TEXT NOT NULL REFERENCES chapter_contents(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, url TEXT NOT NULL, PRIMARY KEY(content_id,position)
);
CREATE TABLE engine_variables (
    book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, chapter_id TEXT,
    name TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(book_id,chapter_id,name),
    FOREIGN KEY(book_id,chapter_id) REFERENCES chapters(book_id,id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX book_variable ON engine_variables(book_id,name) WHERE chapter_id IS NULL;
CREATE TABLE reading_progress (
    book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE, chapter_id TEXT,
    chapter_index INTEGER, offset INTEGER NOT NULL DEFAULT 0, page_id TEXT, anchor TEXT, quality TEXT,
    updated_at_ms INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE library_books (
    book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE, position INTEGER NOT NULL
);
CREATE TABLE library_groups (name TEXT PRIMARY KEY, position INTEGER NOT NULL);
CREATE TABLE library_group_books (
    group_name TEXT NOT NULL REFERENCES library_groups(name) ON DELETE CASCADE,
    book_id TEXT NOT NULL REFERENCES library_books(book_id) ON DELETE CASCADE,
    PRIMARY KEY(group_name,book_id)
);
CREATE TABLE bookmarks (
    id TEXT PRIMARY KEY, book_id TEXT NOT NULL, position INTEGER NOT NULL,
    chapter_id TEXT, chapter_index INTEGER, chapter_title TEXT, offset INTEGER,
    page_id TEXT, anchor TEXT, note TEXT, orphaned BOOLEAN NOT NULL DEFAULT 0,
    created_at_ms INTEGER, updated_at_ms INTEGER
);
CREATE INDEX bookmark_book ON bookmarks(book_id,position);
CREATE TABLE reading_sessions (
    session_id TEXT PRIMARY KEY, book_id TEXT NOT NULL, duration_ms INTEGER NOT NULL,
    ended_at_ms INTEGER NOT NULL, utc_offset_minutes INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX session_book ON reading_sessions(book_id,ended_at_ms);
CREATE TABLE search_history (
    normalized_query TEXT PRIMARY KEY, query TEXT NOT NULL, usage INTEGER NOT NULL,
    first_use_time_ms INTEGER NOT NULL, last_use_time_ms INTEGER NOT NULL
);
CREATE TABLE replacement_rules (
    id TEXT PRIMARY KEY, position INTEGER NOT NULL, name TEXT NOT NULL, pattern TEXT NOT NULL,
    replacement TEXT NOT NULL, enabled BOOLEAN NOT NULL, is_regex BOOLEAN NOT NULL, scope TEXT NOT NULL
);
CREATE TABLE txt_toc_rules (
    id TEXT PRIMARY KEY, position INTEGER NOT NULL, name TEXT NOT NULL, rule TEXT NOT NULL,
    example TEXT, serial_number INTEGER NOT NULL, enable BOOLEAN NOT NULL
);
CREATE TABLE media_requests (
    id TEXT PRIMARY KEY, book_id TEXT REFERENCES books(id) ON DELETE SET NULL, url TEXT NOT NULL
);
CREATE TABLE media_headers (
    owner_id TEXT NOT NULL REFERENCES media_requests(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, name TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(owner_id,position)
);
CREATE TABLE discovery_categories (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    category_id TEXT, title TEXT NOT NULL, kind TEXT, url TEXT, request_kind TEXT,
    style_cols INTEGER, style_rows INTEGER, style_width REAL,
    PRIMARY KEY(source_id,position), UNIQUE(source_id,category_id)
);
CREATE TABLE discovery_favorites (
    source_id TEXT NOT NULL, category_id TEXT NOT NULL, position INTEGER NOT NULL,
    source_name TEXT, title TEXT, kind TEXT, created_at_ms INTEGER,
    style_cols INTEGER, style_rows INTEGER, style_width REAL, PRIMARY KEY(source_id,category_id)
);
CREATE TABLE rss_subscriptions (source_id TEXT PRIMARY KEY, filter TEXT NOT NULL);
CREATE TABLE rss_articles (
    source_id TEXT NOT NULL, article_id TEXT NOT NULL, is_read BOOLEAN NOT NULL,
    is_favorite BOOLEAN NOT NULL, updated_at_ms INTEGER NOT NULL, content_ref TEXT,
    PRIMARY KEY(source_id,article_id)
);
CREATE TABLE source_kv (
    namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, expires_at_ms INTEGER,
    PRIMARY KEY(namespace,key)
);
CREATE INDEX source_expiry ON source_kv(expires_at_ms) WHERE expires_at_ms IS NOT NULL;
CREATE TABLE pending_cleanup (path TEXT PRIMARY KEY);

-- 展示和统计由关系查询派生。
CREATE VIEW source_records AS SELECT s.*,
    CASE WHEN s.is_rss=0 AND coalesce(o.book_source_type,0) IN (0,1,2,4) THEN s.content_type END AS search_type,
    CASE WHEN s.content_type IN ('audio','video') THEN s.content_type END AS media_type,
    (length(trim(coalesce(o.search_url,'')))>0 OR EXISTS(SELECT 1 FROM source_rules r WHERE r.source_id=s.id AND r.stage='ruleSearch')) AS can_search,
    EXISTS(SELECT 1 FROM source_rules r WHERE r.source_id=s.id AND r.stage='ruleBookInfo') AS can_detail,
    EXISTS(SELECT 1 FROM source_rules r WHERE r.source_id=s.id AND r.stage='ruleToc') AS can_toc,
    EXISTS(SELECT 1 FROM source_rules r WHERE r.source_id=s.id AND r.stage='ruleContent') AS can_content
    FROM sources s LEFT JOIN source_options o ON o.owner_id=s.id;
CREATE VIEW chapter_records AS SELECT c.*,b.text_url AS url,b.duration_ms,b.protocol
    FROM chapters c LEFT JOIN chapter_contents b ON b.id=c.content_id;
CREATE VIEW chapter_positions AS SELECT c.book_id,c.id,c.position,
    (SELECT count(*) FROM chapters r WHERE r.book_id=c.book_id AND r.position<=c.position AND r.volume=0) AS read_position
    FROM chapters c;
CREATE VIEW chapter_pages AS SELECT c.book_id,c.id AS chapter_id,p.position,p.id,p.url,p.width,p.height
    FROM chapters c JOIN content_pages p ON p.content_id=c.content_id;
CREATE VIEW chapter_streams AS SELECT c.book_id,c.id AS chapter_id,p.position,p.label,p.url,p.protocol,p.format,p.unavailable_reason,p.is_default
    FROM chapters c JOIN content_streams p ON p.content_id=c.content_id;
CREATE VIEW alternative_sources AS SELECT a.book_id,a.position,b.id AS alternative_id,
    b.source_id,b.book_url AS engine_url,s.name AS source_name,b.title,b.author,b.cover,b.latest_chapter_title AS latest
    FROM book_alternatives a JOIN books b ON b.id=a.alternative_id LEFT JOIN sources s ON s.id=b.source_id;
CREATE VIEW book_records AS SELECT b.*,s.name AS source_name,s.source_group,
    CASE WHEN b.content_type='pdf' THEN coalesce(b.page_count,0) ELSE (SELECT count(*) FROM chapters c WHERE c.book_id=b.id) END AS chapter_count,
    CASE WHEN b.content_type='pdf' THEN coalesce(b.page_count,0) ELSE (SELECT count(*) FROM chapters c WHERE c.book_id=b.id AND c.volume=0) END AS readable_count,
    coalesce(b.latest_chapter_title,(SELECT c.title FROM chapters c WHERE c.book_id=b.id AND c.volume=0 ORDER BY c.position DESC LIMIT 1)) AS latest_title
    FROM books b LEFT JOIN sources s ON s.id=b.source_id;
CREATE VIEW book_previews AS SELECT b.id,b.content_type AS kind,coalesce(b.title_override,b.title) AS title,
    coalesce(b.author_override,b.author,'') AS author,coalesce(b.cover_override,b.cover) AS cover,
    b.source_id,b.source_name,b.chapter_count,b.readable_count,b.latest_title,b.updated_at_ms,
    (b.source_id IS NOT NULL AND b.book_url IS NOT NULL) AS can_change_source,
    p.chapter_id,p.chapter_index,p.page_id,p.anchor,p.quality,coalesce(p.offset,0) AS offset,
    coalesce(p.updated_at_ms,0) AS progress_updated_at_ms
    FROM book_records b LEFT JOIN reading_progress p ON p.book_id=b.id;
CREATE VIEW reading_session_days AS
    WITH RECURSIVE segments(session_id,start_ms,end_ms) AS (
        SELECT session_id,ended_at_ms-duration_ms+utc_offset_minutes*60000,
            ended_at_ms+utc_offset_minutes*60000 FROM reading_sessions WHERE duration_ms>0
        UNION ALL
        SELECT session_id,start_ms-((start_ms%86400000+86400000)%86400000)+86400000,end_ms
        FROM segments WHERE start_ms-((start_ms%86400000+86400000)%86400000)+86400000<end_ms
    ) SELECT session_id,strftime('%Y%m%d',start_ms/1000.0,'unixepoch') AS day,
        min(end_ms,start_ms-((start_ms%86400000+86400000)%86400000)+86400000)-start_ms AS duration_ms FROM segments;
CREATE VIEW reading_books AS SELECT book_id,count(*) AS session_count,sum(duration_ms) AS total_duration_ms,max(ended_at_ms) AS last_read_at_ms FROM reading_sessions GROUP BY book_id;
CREATE VIEW reading_days AS SELECT day,count(*) AS session_count,sum(duration_ms) AS total_duration_ms FROM reading_session_days GROUP BY day;
CREATE VIEW reading_day_books AS SELECT d.day,s.book_id,count(*) AS session_count,sum(d.duration_ms) AS total_duration_ms FROM reading_session_days d JOIN reading_sessions s USING(session_id) GROUP BY d.day,s.book_id;
CREATE VIEW reading_totals AS SELECT 1 AS id,coalesce(sum(duration_ms),0) AS duration_ms,count(*) AS sessions FROM reading_sessions;
PRAGMA user_version=16;

CREATE TABLE content_media (
    content_id TEXT NOT NULL REFERENCES chapter_contents(id) ON DELETE CASCADE,
    media_id TEXT NOT NULL REFERENCES media_requests(id), PRIMARY KEY(content_id,media_id)
);
CREATE VIEW content_assets AS
    SELECT id AS content_id,text_url AS uri FROM chapter_contents WHERE text_url IS NOT NULL
    UNION ALL SELECT content_id,url FROM content_pages
    UNION ALL SELECT content_id,url FROM content_streams WHERE url IS NOT NULL
    UNION ALL SELECT content_id,url FROM content_files;
CREATE VIEW asset_references AS
    SELECT a.uri FROM content_assets a JOIN chapter_contents b ON b.id=a.content_id
    WHERE b.producer_book_id IS NOT NULL OR EXISTS(SELECT 1 FROM chapters c WHERE c.content_id=b.id)
    UNION ALL SELECT CASE WHEN instr(cover,'://')>0 THEN cover ELSE 'resource://cache/'||content_type||'/'||id||'/'||cover END FROM books WHERE cover IS NOT NULL
    UNION ALL SELECT cover_override FROM books WHERE cover_override IS NOT NULL
    UNION ALL SELECT CASE WHEN instr(original,'://')>0 THEN original ELSE 'resource://cache/'||content_type||'/'||id||'/'||original END FROM books WHERE original IS NOT NULL
    UNION ALL SELECT CASE WHEN instr(url,'://')>0 THEN url ELSE 'resource://cache/'||content_type||'/'||id||'/'||url END FROM books WHERE url IS NOT NULL
    UNION ALL SELECT content_ref FROM rss_articles WHERE content_ref IS NOT NULL;
