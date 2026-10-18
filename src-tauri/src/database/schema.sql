-- Business identities and relations. JSON columns contain only opaque engine inputs.
CREATE TABLE IF NOT EXISTS books (
 id TEXT PRIMARY KEY, kind TEXT NOT NULL,
 title TEXT, author TEXT, intro TEXT, cover TEXT, source_id TEXT, source_name TEXT, source_group TEXT,
 word_count TEXT, status TEXT, original_json TEXT, page_count INTEGER, document_url TEXT, updated_at_ms INTEGER,
 instance_id TEXT, catalog_generation TEXT, catalog_version TEXT,
 engine_json TEXT, engine_state_json TEXT,
 title_override TEXT, author_override TEXT, intro_override TEXT, cover_override TEXT,
 chapter_count INTEGER NOT NULL DEFAULT 0, readable_count INTEGER NOT NULL DEFAULT 0, latest_title TEXT
);
CREATE INDEX IF NOT EXISTS book_source ON books(source_id);
CREATE TABLE IF NOT EXISTS book_tags (
 book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, position INTEGER NOT NULL, tag TEXT NOT NULL,
 PRIMARY KEY(book_id,position)
);
CREATE TABLE IF NOT EXISTS chapters (
 book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, id TEXT NOT NULL, position INTEGER NOT NULL,
 read_position INTEGER NOT NULL, title TEXT, is_volume INTEGER, resource_url TEXT, engine_url TEXT,
 engine_title TEXT, engine_data_json TEXT, duration_ms INTEGER, protocol TEXT,
 PRIMARY KEY(book_id,id), UNIQUE(book_id,position)
);
CREATE INDEX IF NOT EXISTS chapter_order ON chapters(book_id,position);
CREATE TABLE IF NOT EXISTS chapter_pages (
 book_id TEXT NOT NULL, chapter_id TEXT NOT NULL, position INTEGER NOT NULL, id TEXT NOT NULL,
 url TEXT, width INTEGER, height INTEGER,
 PRIMARY KEY(book_id,chapter_id,position),
 FOREIGN KEY(book_id,chapter_id) REFERENCES chapters(book_id,id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS chapter_streams (
 book_id TEXT NOT NULL, chapter_id TEXT NOT NULL, position INTEGER NOT NULL,
 label TEXT, url TEXT, protocol TEXT, headers_json TEXT,
 PRIMARY KEY(book_id,chapter_id,position),
 FOREIGN KEY(book_id,chapter_id) REFERENCES chapters(book_id,id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS chapter_files (
 book_id TEXT NOT NULL, chapter_id TEXT NOT NULL, position INTEGER NOT NULL, filename TEXT NOT NULL,
 PRIMARY KEY(book_id,chapter_id,position),
 FOREIGN KEY(book_id,chapter_id) REFERENCES chapters(book_id,id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS reading_progress (
 book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE, chapter_id TEXT, chapter_index INTEGER,
 page_id TEXT, anchor TEXT, quality TEXT, char_offset INTEGER, time_ms INTEGER, offset_ratio REAL, page INTEGER,
 updated_at_ms INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS alternative_sources (
 book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, source_id TEXT NOT NULL, engine_url TEXT NOT NULL,
 position INTEGER NOT NULL, source_name TEXT, title TEXT, author TEXT, cover TEXT, latest TEXT, engine_json TEXT,
 PRIMARY KEY(book_id,source_id,engine_url)
);
CREATE INDEX IF NOT EXISTS alternative_order ON alternative_sources(book_id,position);
CREATE TABLE IF NOT EXISTS media_requests (
 id TEXT PRIMARY KEY, book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
 url TEXT, headers_json TEXT, media_type TEXT, media_format TEXT, request_json TEXT
);
CREATE TABLE IF NOT EXISTS sources (
 id TEXT PRIMARY KEY, position INTEGER NOT NULL, name TEXT NOT NULL, source_group TEXT, enabled INTEGER NOT NULL,
 is_rss INTEGER NOT NULL, content_type TEXT NOT NULL, search_type TEXT, media_type TEXT,
 can_search INTEGER NOT NULL, can_detail INTEGER NOT NULL, can_toc INTEGER NOT NULL, can_content INTEGER NOT NULL,
 user_agent_override TEXT, definition_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS source_selection ON sources(enabled,search_type,position);
CREATE TABLE IF NOT EXISTS library_books (
 book_id TEXT PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE, position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS library_groups (name TEXT PRIMARY KEY, position INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS library_group_books (
 group_name TEXT NOT NULL REFERENCES library_groups(name) ON DELETE CASCADE,
 book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE, position INTEGER NOT NULL,
 PRIMARY KEY(group_name,book_id)
);
CREATE TABLE IF NOT EXISTS reading_sessions (
 session_id TEXT PRIMARY KEY, book_id TEXT NOT NULL, position INTEGER NOT NULL,
 duration_ms INTEGER NOT NULL, ended_at_ms INTEGER NOT NULL, utc_offset_minutes INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS session_book ON reading_sessions(book_id,ended_at_ms);
CREATE TABLE IF NOT EXISTS reading_books (
 book_id TEXT PRIMARY KEY, position INTEGER NOT NULL, session_count INTEGER NOT NULL,
 total_duration_ms INTEGER NOT NULL, last_read_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS reading_days (
 day TEXT PRIMARY KEY, position INTEGER NOT NULL, session_count INTEGER NOT NULL, total_duration_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS reading_day_books (
 day TEXT NOT NULL REFERENCES reading_days(day) ON DELETE CASCADE, book_id TEXT NOT NULL, position INTEGER NOT NULL,
 session_count INTEGER NOT NULL, total_duration_ms INTEGER NOT NULL, PRIMARY KEY(day,book_id)
);
CREATE TABLE IF NOT EXISTS search_history (
 normalized_query TEXT PRIMARY KEY, query TEXT NOT NULL, position INTEGER NOT NULL, usage INTEGER NOT NULL,
 first_use_time_ms INTEGER NOT NULL, last_use_time_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS recorded_searches (id TEXT PRIMARY KEY, position INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS search_recency ON search_history(last_use_time_ms DESC);
CREATE TABLE IF NOT EXISTS bookmarks (
 id TEXT PRIMARY KEY, book_id TEXT NOT NULL, position INTEGER NOT NULL, chapter_id TEXT, chapter_index INTEGER,
 chapter_title TEXT, offset INTEGER, page_id TEXT, anchor TEXT, note TEXT, orphaned INTEGER,
 created_at_ms INTEGER, updated_at_ms INTEGER
);
CREATE INDEX IF NOT EXISTS bookmark_book ON bookmarks(book_id,position);
CREATE TABLE IF NOT EXISTS tasks (
 id TEXT PRIMARY KEY, position INTEGER NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL, book_id TEXT,
 keyword TEXT, source_type TEXT, precision_search INTEGER, page INTEGER, from_index INTEGER, total INTEGER,
 completed INTEGER, check_only INTEGER, search_id TEXT, result_json TEXT, error TEXT,
 created_at_ms INTEGER, updated_at_ms INTEGER
);
CREATE TABLE IF NOT EXISTS task_chapters (
 task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, position INTEGER NOT NULL, chapter_id TEXT NOT NULL,
 PRIMARY KEY(task_id,position)
);
CREATE TABLE IF NOT EXISTS task_sources (
 task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, position INTEGER NOT NULL, source_id TEXT NOT NULL,
 PRIMARY KEY(task_id,position)
);
CREATE TABLE IF NOT EXISTS rss_subscriptions (source_id TEXT PRIMARY KEY, position INTEGER NOT NULL, filter TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS rss_articles (
 source_id TEXT NOT NULL, article_id TEXT NOT NULL, position INTEGER NOT NULL,
 is_read INTEGER NOT NULL, is_favorite INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, content_ref TEXT,
 PRIMARY KEY(source_id,article_id)
);
CREATE TABLE IF NOT EXISTS discovery_favorites (
 source_id TEXT NOT NULL, category_id TEXT NOT NULL, position INTEGER NOT NULL, source_name TEXT, title TEXT,
 kind TEXT, style_json TEXT, created_at_ms INTEGER, PRIMARY KEY(source_id,category_id)
);
CREATE TABLE IF NOT EXISTS source_kv (
 namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, expires_at_ms INTEGER,
 PRIMARY KEY(namespace,key)
);
CREATE INDEX IF NOT EXISTS source_kv_expiry ON source_kv(expires_at_ms) WHERE expires_at_ms IS NOT NULL;
CREATE TABLE IF NOT EXISTS pending_cleanup (path TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS reading_totals (id INTEGER PRIMARY KEY CHECK(id=1), duration_ms INTEGER NOT NULL, sessions INTEGER NOT NULL);
INSERT OR IGNORE INTO reading_totals VALUES(1,0,0);
CREATE TABLE IF NOT EXISTS replacement_rules (
 id TEXT PRIMARY KEY, position INTEGER NOT NULL, name TEXT NOT NULL, pattern TEXT NOT NULL,
 replacement TEXT NOT NULL, enabled INTEGER NOT NULL, is_regex INTEGER NOT NULL, scope TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS txt_toc_rules (
 id TEXT PRIMARY KEY, position INTEGER NOT NULL, name TEXT NOT NULL, rule TEXT NOT NULL,
 example TEXT, serial_number INTEGER NOT NULL, enabled INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS http_tts_configs (
 id TEXT PRIMARY KEY, position INTEGER NOT NULL, name TEXT NOT NULL, enabled INTEGER NOT NULL,
 definition_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS discovery_sources (
 source_id TEXT NOT NULL, private INTEGER NOT NULL, PRIMARY KEY(source_id,private)
);
CREATE TABLE IF NOT EXISTS discovery_categories (
 source_id TEXT NOT NULL, private INTEGER NOT NULL, position INTEGER NOT NULL, category_id TEXT,
 title TEXT NOT NULL, kind TEXT, style_json TEXT, url TEXT,
 PRIMARY KEY(source_id,private,position),
 FOREIGN KEY(source_id,private) REFERENCES discovery_sources(source_id,private) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS discovery_pages (
 id TEXT PRIMARY KEY, keyword TEXT, page INTEGER, complete INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS discovery_page_items (
 page_id TEXT NOT NULL REFERENCES discovery_pages(id) ON DELETE CASCADE, position INTEGER NOT NULL,
 result_id TEXT, article_id TEXT, source_id TEXT, source_name TEXT, title TEXT, author TEXT,
 intro TEXT, cover TEXT, url TEXT, content_src TEXT, published_at TEXT, is_read INTEGER, is_favorite INTEGER,
 PRIMARY KEY(page_id,position)
);
CREATE VIEW IF NOT EXISTS book_previews AS
 SELECT b.id,b.kind,coalesce(b.title_override,b.title) AS title,
 coalesce(b.author_override,b.author,'') AS author,coalesce(b.cover_override,b.cover) AS cover,
 b.source_id,b.source_name,b.chapter_count,b.readable_count,b.latest_title,b.updated_at_ms,
 (b.source_id IS NOT NULL AND b.engine_json IS NOT NULL) AS can_change_source,
 p.chapter_id,p.chapter_index,p.page_id,p.anchor,p.quality,
 CASE b.kind WHEN 'image' THEN cast(coalesce(p.offset_ratio,0)*1000000 AS INTEGER)
 WHEN 'audio' THEN coalesce(p.time_ms,0) WHEN 'video' THEN coalesce(p.time_ms,0)
 ELSE coalesce(p.char_offset,0) END AS offset,coalesce(p.updated_at_ms,0) AS progress_updated_at_ms
 FROM books b LEFT JOIN reading_progress p ON p.book_id=b.id;
PRAGMA user_version=1;
