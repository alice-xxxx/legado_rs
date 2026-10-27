# 存储

业务库为 SQLite `library.sqlite3`，结构版本 16。固定字段用普通列，有序集合用关系子表；布尔列声明为 `BOOLEAN`。不保存 JSON 对象，不使用 SQL JSON 函数。旧库不迁移，版本不匹配直接报错。

## 边界

`db.rs` 是唯一能创建连接、执行语句、提交事务的文件；其 `read/write` 仅供 database 内部使用。固定写入通过 `Mutation` 枚举和 `Database::submit` 提交；外部不需要知道表结构或 SQL。

- Rust 的连接、SQL、事务和数据拆装只在 `src-tauri/src/database` 内，其它模块调用固定业务 API，不能传 SQL、表名、字段映射或事务闭包，也不能取得连接。
- Vue 通过 SQL 插件 `sqlite:library-reader` 自行读取，连接物理只读，权限只开放 `select`；Rust 查询也使用只读连接。组合读取使用同一查询或同一读事务的快照。
- 一个写线程持有唯一写连接，通过有界队列串行提交事务。WAL 支持并发读取；业务没有共享读写连接锁，SQLite 仍使用自身事务锁。
- 文件下载、格式解析、播放在业务模块完成，数据库登记结果。`settings.json` 只保存独立应用设置。

## 关系结构

| 数据 | 表与关系 |
| --- | --- |
| 书源 | `sources` 保存身份和状态；`source_options` 保存固定选项；`source_rules/headers/login_items/login_choices/explore_links` 保存规则、请求头、表单、选项和发现链接。`source_records` 视图派生能力与搜索分类。 |
| TTS | `http_tts_configs` 保存身份；`http_tts_options/headers/login_items/login_choices` 分别保存选项和子项，与书源分开。 |
| 书籍 | `books` 保存资料、用户覆盖和引擎固定字段；`information_fetched_at_ms` 单独记录详情与目录最后成功获取时间，`updated_at_ms` 记录目录变化时间；`book_tags` 保存标签；`book_alternatives` 通过书籍 ID 关联候选。 |
| 目录与变量 | `chapters` 以 `(book_id,id)` 标识章节；`engine_variables` 以书籍、可空章节 ID 和变量名区分作用域，随所属记录删除；引擎明确提供变量表时整体替换，缺省时保留。 |
| 正文 | `chapter_contents` 保存生成关系和正文地址；`content_pages/streams/files/media` 保存图片页、线路和附属文件。`chapters.content_id` 指向当前正文，允许跨来源借用。 |
| 书架 | `library_books` 保存顺序；`library_groups` 保存分组；`library_group_books` 保存成员关系。 |
| 阅读 | `reading_progress.offset` 保存文字偏移、毫秒或百万分比，PDF 页使用章节序号；`bookmarks` 保存书签；`reading_sessions` 按会话 ID 去重，跨日时长和统计由视图派生。 |
| 发现与 RSS | `discovery_categories` 同行保存展示字段和请求目标；`discovery_favorites` 保存收藏；`rss_subscriptions/articles` 保存订阅和文章状态。分页结果直接返回，不另存文档。 |
| 其他 | `replacement_rules`、`txt_toc_rules` 保存规则；`media_requests/headers` 保存资源请求；`source_kv` 服务插件的原生字符串键值协议；`pending_cleanup` 保存清理候选路径，删除前统一复核引用。 |

编辑器和引擎协议只在边界临时组装。Vue 组合展示数据；Rust 的 `book` 读取运行所需的书籍、位置和章节，按内容类型读取子表。资源文件放在 `cache/{类型}/{id}/`，资料、目录和阅读位置都在数据库中。

正文发布插入新 ID 和子行后切换引用，被借用的旧正文不能覆盖。删除生成章节或清理缓存只解除相关关系；仍被引用的正文和文件继续保留。资源路径按存储值使用。删除文件前复核业务表和设置引用，不递归删除整本书的缓存目录；清理失败保留候选记录重试，不把已提交的资料删除报告为写入失败。

## API 与回执

资源由 `resources` 独立提供启动数据、缓存读写与清理统计、代理登记与恢复、TTS 和 HTTP 生命周期；外部只调用 API。`resources/io.rs` 统一鉴权并分发 `cache/{类型}/{id}/{文件}`、`settings.json`、`proxy/{id}`、`tts/{id}`，HTTP 只读。文件写入共用按文件协调的原子替换；代理通过具名数据库 API 保存并按原 ID 恢复，TTS 是临时流。资源与应用共享数据库句柄，资源模块不依赖应用服务；正文转换属于正文业务。

Vue 只有 `fe_db(operations, prepareCurrent, completed)` 一个写入口；固定操作列表在一个事务内执行，失败整批回滚。返回表示已入队；独立完成通道在提交或回滚后返回 `{ error }`，为空表示成功。`modifyDatabase` 默认等待入队，`wait: true` 等待提交，`onComplete` 接收完成通知。

Rust 写入返回 `Pending<T>`：普通调用入队后即可继续；依赖提交结果时直接 `.await`，同步协议使用 `.wait()`。通知丢弃不会取消已入队操作。

| 接口 | 用途 |
| --- | --- |
| Vue `saveSources` | 统一创建、编辑、启停、分组和标记移除。带定义时整体替换，不带定义时只修改书源状态行。 |
| Vue `saveBooks` / `saveBook` | 保存在线候选；修改用户覆盖、标签、候选关系和 PDF 页数。 |
| Vue `useChapterContent` / `saveProgress` / `resetProgress` | 切换正文引用；保存或清空阅读位置。 |
| Vue `saveShelf` | 替换书架顺序、分组和成员。 |
| Vue `saveReplacementRules` / `saveTxtTocRules` / `saveHttpTts` | 按 ID 保存配置；对应 `deleteReplacementRule` / `deleteTxtTocRule` / `deleteHttpTts` 按 ID 删除。 |
| Vue `saveBookmark` / `deleteBookmark` / `recordSession` / `clearSearchHistory` | 保存书签、会话；删除书签、一条或全部搜索历史。 |
| Rust `import_book` / `shelve_import` / `bind_book_type` / `save_information` / `save_main_text` | 导入、上架、绑定内容类型、刷新目录和发布正文。引擎上下文按实际提供的字段保存；目录匹配、缓存保留及位置映射在数据库内完成。 |
| Rust `save_discovery_categories` / `ensure_rss_subscription` / `save_rss_articles` | 保存分类、登记订阅；批量登记文章或修改已登记文章的标记、正文地址。 |
| Rust `save_media` / `clear_chapter_cache` / `remove_book_assets` / `recover_cleanup` | 登记资源请求，清理缓存、书籍及待删文件。 |
| Rust `backup_file` / `restore_archive` / `storage` | 通过同一写线程备份、恢复；执行固定的原生 KV 操作。 |

Rust 查询使用 `sources`、`book`、`book_engine`、`book_type`、`txt_toc_rules`、`tts_config`、`rss_state`、`discovery_category`、`media`、`asset_referenced` 等具名接口。在线书籍统一使用 `online_book_id` 生成身份。

部分修改中，缺省字段保留，允许清空的字段用 `null` 清空；明确提交的有序子列表整体替换。阅读位置的章节归属和时间戳在库内处理。提交后才发送书源、进度通知，依赖新值的调用方再查询。`prepareCurrent` 可请求预读；回执只表示数据库结果，后续资源失败通过 `reading-resource-error` 通知。
