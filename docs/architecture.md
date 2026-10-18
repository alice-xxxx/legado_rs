# 架构与数据

## 职责

| 层 | 职责 |
|---|---|
| Vue | 页面交互、共享响应式状态、直接查询只读 SQLite、排版与播放。 |
| Rust | 应用服务、数据库写入、任务、缓存、文件资源服务与引擎宿主能力。 |
| KMP | 执行书源规则和脚本。桌面通过 JVM/JNI，Android/iOS 通过原生插件接入。 |

修改操作通过 Tauri IPC 提交。Vue 使用官方 SQL 插件读取业务表，Rust 在启动时注册只读连接池，仅授予 `sql:allow-select`，不向前端开放建库、加载连接或执行写 SQL。设置通过启动及保存命令直接返回数据。搜索通过 Tauri Channel 发送临时结果。正文、图片和媒体使用 HTTP 文件资源；业务数据没有 `.json` HTTP 资源接口。

## 代码入口

| 路径 | 用途 |
|---|---|
| `src/main.ts`、`src/app/RoutesRoot.vue`、`src/router/` | 初始化共享状态、挂载页面及独立功能 host、哈希路由。 |
| `src/features/` | 页面、阅读器、导入、配置及对应状态。 |
| `src/api/`、`src/ui/` | 类型化命令与资源接口、共用展示组件。 |
| `src-tauri/src/application/` | ApplicationService 和 Tauri 命令。 |
| `src-tauri/src/books/`、`sources/`、`discovery/`、`rss/` | 本地书籍、书源引擎、发现与 RSS。 |
| `src-tauri/src/resources/`、`backup/`、`models/` | 文件及 HTTP 资源、事务恢复、备份、数据类型。 |
| `src-tauri/src/reading_tools/`、`search/`、`settings/` | 阅读记录、规则、搜索历史与朗读配置。 |
| `kotlin/`、`src-tauri/plugins/source-engine/` | KMP 引擎、原生库及移动端插件。 |

## 持久化

Android 业务数据位于应用私有目录的 `files/legado/`，其他平台使用 Tauri 应用数据目录。跨进程锁和备份恢复暂存目录位于业务数据目录旁；Android 上均保留在应用沙箱内。字段定义以 Rust 模型和 `src/api/types.ts` 为准。

| 文件或目录 | 内容 |
|---|---|
| `settings.json` | 应用设置，独立原子写入并保留用户现有配置。 |
| `library.sqlite3` | 书源、内容资料与章节索引、进度、内容库、历史、书签、规则、HTTP TTS、任务、RSS、发现分类和引擎键值数据。 |
| `library.sqlite3-wal`、`library.sqlite3-shm` | SQLite 写入日志和共享内存文件，由 SQLite 管理。 |
| `cache/{类型}/{id}/` | 正文、图片、媒体、封面和导入原文件。 |
| `source-engine/` | 引擎运行时文件。 |

业务数据使用独立表和明确字段：`books`、`chapters`、`chapter_pages`、`reading_progress`、`sources`、`library_books` 及分组关系、历史、书签、任务、规则和发现分类等。没有通用文档、字段或记录存储表。书源规则与引擎输入等不透明载荷有专用 JSON 列，业务身份、顺序、状态和关系单独建模。表与索引定义见 `src-tauri/src/database/schema.sql`。

一个 Rust 写入线程持有写连接，调用方发送修改命令，数据库连接不由业务代码共享加锁。Vue 和 Rust 查询使用独立只读 WAL 连接。进度、媒体、显示覆盖、单条规则及单章缓存分别更新对应行，不重写整本目录。搜索历史、书签、阅读会话和任务检查点分别更新所属表及行；任务状态更新保留原有章节、书源快照。网络请求不占用数据库写入线程；文件发布和同章请求去重仍有各自的协调机制。

旧业务 JSON 和旧 `app.sqlite3` 不自动迁移，也不作为运行时回退。备份在同一数据库快照内导出交换格式及文件资源，显式恢复时重建业务表；设置保持为文件。恢复前关闭前端只读池并排空 Rust 写入连接，恢复后重新绑定只读池。

### 内容库与缓存

```text
app-data/
├─ settings.json
├─ library.sqlite3
└─ cache/
   └─ {text|html|image|audio|video|pdf}/
      └─ {id}/
         ├─ original.*
         └─ 正文、封面和媒体资源
```

内容 ID 标识一份缓存，不按书名共用目录。资料、目录、进度及引擎状态保存在数据库中，目录内资源平铺；TXT/HTML 正文文件名由章节与内容哈希生成。

内容库成员和分组关系保存在独立表。Vue 联查 `book_previews`、`library_books` 和分组表，生成按内容类型组织的视图；`readAt`、`updatedAt` 使用秒。

预览从资料和进度联查生成，不保存第二份书籍资料。移出内容库或删除分组保留缓存，不在启动时按内容库成员清理缓存。普通详情与阅读入口不自动上架，本地导入完成后自动上架。

| 类型 | 磁盘内容 | 阅读视图 |
|---|---|---|
| text | 章节 `url` 引用同目录 `.txt`。 | 纯文本。 |
| html | 章节 `url` 引用 `.html` 及同目录资源。 | 净化后的 HTML。 |
| image | 章节 `pages` 保存页 ID 和 `url`。 | 有序图片页。 |
| audio / video | 章节入口、可选 `sources` 和本地依赖索引。 | 媒体播放；HLS 使用播放列表及其依赖。 |
| pdf | 文档 `url`、`pageCount` 和 `original.pdf`。 | PDF.js 读取原文档。 |

引擎上下文保存在书籍、章节的专用引擎载荷列中，媒体请求映射保存在 `media_requests`。Vue 的展示查询选择所需业务字段，不加载引擎上下文；正文与播放入口按文件索引构建。

磁盘资源使用文件名、稳定的 `resource://` 引用或远程地址。HTTP 响应生成运行时 `src`，端口和访问 token 不持久化；远程或代理播放不等于已离线缓存。显示替换和阅读样式不回写正文。

### 阅读进度

阅读位置通过 `notify_reading` 保存，`reading-progress-updated` 增量更新前端进度；仅切换章节或显式刷新时触发预加载，准备的章节通过 `book-updated` 发布。数据库进度包含 `updatedAtMs`，位置按类型记录：

| 类型 | 位置 |
|---|---|
| text / html | `chapterId`、`charOffset`；HTML 可带段落 `anchor`。 |
| image | `chapterId`、`pageId`、`offsetRatio`。 |
| audio / video | `chapterId`、`timeMs`；视频可带 `quality`。 |
| pdf | 从 1 开始的 `page`。 |

前端的统一进度来自 `book_previews` 视图，读取实现见 `src/api/database.ts`；写入转换由 Rust 完成。

## Vue 共享状态

`src/main.ts` 初始化 `src/features/` 下的共享状态：设置由 `settingsStore.ts` 管理，历史和书签由 `historyStore.ts` 管理，规则和 HTTP TTS 由 `configurationStores.ts` 管理。内容库使用 `contentLibraryState.ts`，书架模块提供派生视图；`sourcesStore.ts` 和 `tasksStore.ts` 分别管理书源与任务。

详情与阅读共用 `bookDetailState.ts` 的 `openedBook` 和轻量 `bookPreviews`。Vue 直接查询预览视图，按需加载完整资料；书籍通过 `bookId` 标识。一次 SQL 查询读取完整目录及其关联表，保持同一 WAL 快照，列表不读取完整目录。

来源列表、详情目录和阅读器目录按可见区域渲染，筛选、选择和定位仍作用于完整数据。弹层统一管理键盘焦点、返回焦点和背景滚动，动画遵循系统的减少动态效果设置。打开章节会显式检查正文是否准备好，即使仍是上次阅读的同一章；普通进度保存不重复准备目录和正文。

Vue 修改共享状态或提交编辑字段，Rust 持久化并通知回读。编辑书籍只提交显示覆盖字段；完整书源规则仅保存在编辑草稿中。内容库排序保存在 WebView 的 localStorage，分组和成员写入数据库。换源时替换已有成员的预览，非成员只切换当前内容。

## 本地导入

Rust 解析并持久化，Vue 展示缓存。导入按内容、格式和解析选项生成 `local-{hash}` ID，保存原文件、资源与索引后加入内容库。相同内容与选项复用缓存，保留进度、显示覆盖和分组；原路径和文件名不参与 ID。解析失败清理未完成缓存，上架失败保留已生成的缓存并报错。

- **TXT**：缓存为 `text`，支持 UTF-8、带 BOM 的 UTF-16、GBK 和显式字符集。使用启用的目录规则，无配置时使用内置识别；无匹配或全部禁用时按正文分段。标题不重复写入正文。
- **EPUB**：缓存为 `html`，读取 EPUB 3 导航或 EPUB 2 NCX，按 spine 与片段边界分章，目录不可用时回退到 spine。提取元数据并重写本地资源链接；使用阅读设置，段落默认首行缩进两字。
- **CBZ**：缓存为 `image`，图片自然排序，已有 WebP 校验后复用，其他格式转为无损 WebP。存在 ComicInfo.xml 时读取元数据。
- **PDF**：缓存为 `pdf`，保留原文件和页数，运行时生成页索引。加密文件通过临时令牌重试密码，密码不持久化。

入口为 `src-tauri/src/books/local_books.rs`、`src-tauri/src/application/file_import.rs` 和 `src/features/books/useLocalBookImport.ts`。

## 搜索与深度链接

`start_search` 支持 `text`、`audio`、`image`、`video`。结果通过 Tauri Channel 增量发送，`get_search_task_results` 读取内存结果。`resultId` 是候选标识；打开候选后准备完整资料，其他来源保存在 `alternatives`，实际换源时才准备目标缓存。释放搜索任务不删除已生成的缓存。

每轮搜索使用开始执行时取得的来源定义。搜索结果返回后不重新检查来源是否修改，也不计算来源指纹；详情和换源候选直接使用调用时取得的来源，不因来源编辑而拒绝已有结果。修改来源后由用户重新搜索。

正文准备、下载和显式刷新也不在返回时比较来源定义。换源提交使用已有的内容实例和目录标识检查目标，不重复哈希整份目录。读取书籍只返回资源描述符，资料更新通过显式刷新完成。配置条目的更新和删除直接提交对应条目，不通过深度监听和全列表 JSON 序列化推断变更。

普通搜索和换源搜索共用执行额度：移动端最多 4 个源，桌面端最多 8 个源。详情准备期间，每个搜索任务将并发目标降到 1，已有请求结束后逐步让出容量。搜索复用分组索引，按 250 毫秒批次排序并发送变化项，结束时发送剩余结果。

详情页先显示搜索结果已有的资料，来源列表持续接收同组搜索结果，加载中也可换源。新选择或关闭页面会取消旧详情请求；已有完整缓存直接从 SQLite 复用，不重复请求信息和目录。候选来源单独更新数据库字段，不读取或重写章节目录。

书架书籍在详情的「来源」页直接展示缓存候选；补充搜索通过独立 Channel 推送新增候选，搜索期间保留已有列表并允许选择，不等待全部来源结束。搜索、换源和指定章节正文替换均在该页操作，离开详情或成功换源后停止候选搜索；作者缺失时保留身份确认。

阅读器与详情共用来源列表和目录组件。来源展示封面、书名、作者及已有的最新章节；阅读器以完整页面承载来源和目录，返回后继续阅读。目录共用搜索、正倒序、当前章节定位、缓存和指定章节换源操作，并按可见区域渲染章节。

Tauri 注册 `legado`、`yuedu`、`legado-rs` 协议。解析与处理入口为 `src/features/import/deepLinks.ts`、`useExternalImportActions.ts`，冷启动、运行中唤起和粘贴导入共用队列与弹窗。

`legado`、`yuedu` 使用第一个 `src` 查询参数，缺失或为空时忽略：

| 路径 | 行为 |
|---|---|
| `/bookSource`、`/rssSource` | 书源或 RSS 配置勾选导入。 |
| `/replaceRule`、`/textTocRule`、`/httpTTS` | 对应配置勾选导入。 |
| `/importonline` | 按 host 区分旧式配置，其他情况自动识别。 |
| `/read` | 已在内容库则继续阅读，否则准备书籍并打开详情。 |
| `/addToBookshelf` | 准备详情，由用户决定加入内容库。 |
| `/theme`、`/dictRule`、`/readConfig` | 提示暂未提供对应导入功能。 |
| 其他路径 | 自动识别已有配置类型。 |

`legado-rs://import-source?url=...` 和 `legado-rs://import-rules?url=...` 分别导入书源和替换规则。载荷可为 HTTP(S) 地址或 JSON，由 Rust 下载、识别和校验，确认后保存；书籍链接不读取其指定的本地文件。协议注册见 `src-tauri/tauri.conf.json`。
