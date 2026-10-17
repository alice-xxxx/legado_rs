# 架构与数据

## 职责

| 层 | 职责 |
|---|---|
| Vue | 页面交互、共享响应式状态、读取资源、排版与播放。 |
| Rust | 应用服务、持久化、任务、缓存、资源服务与引擎宿主能力。 |
| KMP | 执行书源规则和脚本。桌面通过 JVM/JNI，Android/iOS 通过原生插件接入。 |

应用命令由 Tauri IPC 调用。书籍和配置以资源描述符交付，Vue 通过 HTTP 读取；搜索使用 Tauri Channel 发送结构化结果。运行时资源地址不写入长期数据。

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
| `app.sqlite3` | 设置、书源、内容资料与章节索引、进度、内容库、历史、书签、规则、任务、RSS、发现分类和引擎键值数据。 |
| `app.sqlite3-wal`、`app.sqlite3-shm` | SQLite 写入日志和共享内存文件，由 SQLite 管理。 |
| `cache/{类型}/{id}/` | 正文、图片、媒体、封面和导入原文件。 |
| `source-engine/` | 引擎运行时文件。 |

应用使用随程序编译的 SQLite。`documents` 保存资源身份，`fields` 保存字段和结构，`records` 保存有序章节、书源和其他集合条目；结构性嵌套对象继续拆分，单个书源或章节的引擎载荷保留在各自记录中。内容 ID、条目 ID 和顺序有索引，`source_kv` 按命名空间和键查询并检查过期时间。

进度、媒体映射、显示覆盖和进度重置使用独立 SQL 更新；修改单个书源仅提交变更记录。普通数据库更新不获取文件写锁。SQLite 事务负责数据库一致性，章节获取、任务状态和文件资源发布仍保留必要的同步。

旧 JSON 数据不自动迁移，也不作为运行时回退数据。`resource://...json` 继续作为接口地址，HTTP 按需生成 JSON。备份在同一数据库快照内导出 JSON 和资源，显式恢复时将校验后的归档写入数据库，不直接复制正在使用的数据库文件。

### 内容库与缓存

```text
app-data/
├─ app.sqlite3
└─ cache/
   └─ {text|html|image|audio|video|pdf}/
      └─ {id}/
         ├─ original.*
         └─ 正文、封面和媒体资源
```

内容 ID 标识一份缓存，不按书名共用目录。资料、目录、进度及引擎状态保存在数据库中，目录内资源平铺；TXT/HTML 正文文件名由章节与内容哈希生成。

内容库接口 `content-library.json` 按内容类型返回预览数组，分组使用 `{name, id: [内容ID]}`。预览包含标题、作者、封面、目录总数、最新标题和阅读摘要；`readAt`、`updatedAt` 使用秒。内容库成员由预览列表决定。

加入内容库时提取预览，更新完整资料时仅刷新已存在的预览。移出内容库或删除分组保留缓存，不在启动时按内容库成员清理缓存。普通详情与阅读入口不自动上架，本地导入完成后自动上架。

| 类型 | 磁盘内容 | 阅读视图 |
|---|---|---|
| text | 章节 `url` 引用同目录 `.txt`。 | 纯文本。 |
| html | 章节 `url` 引用 `.html` 及同目录资源。 | 净化后的 HTML。 |
| image | 章节 `pages` 保存页 ID 和 `url`。 | 有序图片页。 |
| audio / video | 章节入口、可选 `sources` 和本地依赖索引。 | 媒体播放；HLS 使用播放列表及其依赖。 |
| pdf | 文档 `url`、`pageCount` 和 `original.pdf`。 | PDF.js 读取原文档。 |

引擎上下文存于 `engine`、`engineData`、`engineState` 等私有字段，媒体请求映射存于 `media`。Rust 生成公开响应时过滤这些字段，补充章节序号、资源描述符和阅读视图。

磁盘资源使用文件名、稳定的 `resource://` 引用或远程地址。HTTP 响应生成运行时 `src`，端口和访问 token 不持久化；远程或代理播放不等于已离线缓存。显示替换和阅读样式不回写正文。

### 阅读进度

阅读位置通过 `notify_reading` 保存，`reading-progress-updated` 增量更新前端进度；仅切换章节或显式刷新时触发预加载，准备的章节通过 `book-updated` 发布。数据库进度包含 `updatedAtMs`，位置按类型记录：

| 类型 | 位置 |
|---|---|
| text / html | `chapterId`、`charOffset`；HTML 可带段落 `anchor`。 |
| image | `chapterId`、`pageId`、`offsetRatio`。 |
| audio / video | `chapterId`、`timeMs`；视频可带 `quality`。 |
| pdf | 从 1 开始的 `page`。 |

前端使用投影后的统一进度，格式转换见 `src-tauri/src/resources/content.rs`。

## Vue 共享状态

`src/main.ts` 初始化 `src/features/` 下的共享状态：设置由 `settingsStore.ts` 管理，历史和书签由 `historyStore.ts` 管理，规则和 HTTP TTS 由 `configurationStores.ts` 管理。内容库使用 `contentLibraryState.ts`，书架模块提供派生视图；`sourcesStore.ts` 和 `tasksStore.ts` 分别管理书源与任务。

详情与阅读共用 `bookDetailState.ts` 的 `openedBook` 和轻量 `bookPreviews`。列表由 `get_book_preview` 查询元数据和索引，不读取完整目录。完整资料按需加载：先调用 `get_book` 获取描述符，再读取资源。

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
