# 功能盘点与验收矩阵

本清单于 **2026-10-02 首轮源码快照** 根据旧仓库的产品入口、路由、界面和服务/模型代码盘点。它描述需要在 `legado_rs` 中实现并实际验收的能力，不表示旧项目每个旧行为都正确，也不要求保留旧版 bug 或未完成 UI。源码路径相对于对应仓库根目录。`旧版源码依据` 表明需求和原有实现线索，**不单凭 route/页面存在推定旧版各平台功能成熟**；源码可确认的旧版缺失/未完备项另列在“旧版成熟度边界”。矩阵状态描述初始 `legado_rs` 快照，代码演进后需按后续实现记录及时更新；没有功能可因代码“看起来已连上”就标记为端到端完成。

状态含义：**未实现** = 当前没有产品级实现；**部分实现** = 有可复用引擎/宿主底座或测试/演示入口，但用户功能、资源架构或验收链未完整；**已验收** = 需要真实可用流程及证据后才能使用。当前没有任何整项达到“已验收”。下表是 **2026-10-03 当前状态覆盖表**，优先于首轮快照矩阵里尚未更新的状态单元格；首次源码快照及其结论保留作历史，不代表今天全部仍未实现。

## 当前实现与缺口（2026-10-03）

| ID | 当前状态与证据 | 本次提交快照仍缺少的工作 |
|---|---|---|
| APP-01 | 部分实现：Vue 有主导航和多个页面入口 | Rust 持久化可配置首页 tabs/sections；导航和首页展示完整验收 |
| LIB-01 | 部分实现：Rust shelf JSON、排序、每书分组与空分组注册表；Vue 书架页面；分组/排序及重启测试通过 | 批量管理、全部书架交互、跨平台 UI 验收 |
| DISC-01 | 部分实现：Rust/KMP 分类和分页 adapter、私有分类 URL→opaque ID、公开处理后结果资源；Kotlin/JVM 源码编译通过 | KMP runtime 真实分类/分页 fixture、发现 UI、持久收藏/排序、错误/刷新完整流程 |
| SEARCH-01 | 部分实现：Rust/KMP 搜索与浏览器真实搜索运行；添加结果步骤因覆盖层挡住按钮失败（见验证日志） | 修复并跑通搜索→详情→加入书架；分页、历史、筛选、失败恢复 |
| BOOK-01 | 部分实现：搜索结果可创建书架资源；Rust 集成测试跑通详情/目录流程 | 编辑、刷新和完整产品详情 UI 流程 |
| READ-01 | 部分实现：目录 JSON/章节 `src` 与源引擎调用存在 | 目录搜索、换源/章节源、刷新、离线和 UI 完整验收 |
| READ-02 | 部分实现：Rust 集成用例验证源引擎结果写入 HTML、章节资源 URL、重启持久 | cache window/预取体验、认证图片在各平台读取及离线边界验收 |
| READ-03 | 部分实现：Vue 阅读器和展示样式逻辑存在 | 分页/滚动/手势/旋转/大文本等完整体验和各平台验收 |
| READ-04 | 部分实现：Rust 进度 JSON 命令和前端生命周期保存调用存在 | 后台/离开应用/强杀边界、恢复位置和设备级端到端验收 |
| READ-05 | 部分实现：Rust canonical 替换规则 JSON、Vue 编辑和 JS 展示替换逻辑存在 | 只读 HTML/text node-only 实际验收、替换 scope 与旧 settings 字段清理验证 |
| READ-06 | 部分实现：书签/阅读历史 JSON CRUD、幂等 session、本地日统计及重启测试；Vue 有入口 | 端到端书签跳转、离书计时、统计展示、完整读写生命周期验收 |
| SOURCE-01 | 部分实现：Rust 私有来源管理、导入/启停 metadata 与现有 KMP 执行桥存在 | 完整书源编辑/导出/排序/分组/多选与产品 UI 验收 |
| SOURCE-02 | 部分实现：KMP 现有操作可经 Rust 调用 | 产品化调试/帮助/逐规则诊断与用户可理解错误 |
| SOURCE-03 | 部分实现：Rust 引擎 HTTP/storage/Cookie 宿主已有底座 | 用户级登录、UA/代理/请求头配置与移动/桌面实测 |
| SOURCE-04 | 部分实现：保留兼容现有引擎及 QuickJS；图片 `image.*` 处理已知未接入 | 对支持规则格式完整跑兼容矩阵并补宿主能力缺口 |
| IMPORT-01 | 部分实现：Rust 本地 TXT/EPUB 导入、章节 HTML/资源路径实现；当前全套 Rust 测试通过 | PDF/压缩包、平台授权与文件 UI 逐项验收 |
| IMPORT-02 | 未实现：WebDAV/远程书籍导入服务不在当前代码 | 远程浏览/上传下载、凭据、重试/取消和平台测试 |
| RSS-01 | 部分实现：旧 RSS 走既有 KMP adapter；普通 Atom/RSS 由 `feed-rs` 解析。Rust fixture 验证 sanitizer、稳定 `resource://`、重启读取；本次 34 个 Rust tests 全部通过 | 订阅 UI、稳定文章已读/未读/收藏状态、过滤、刷新和退订资源清理 |
| MEDIA-01 | 未实现：资源层有 media proxy，不等同漫画产品流程 | 漫画源、图像目录、缩放/翻页和内存压力验证 |
| MEDIA-02 | 未实现 | TTS/音频引擎接入、后台和平台媒体控制 |
| MEDIA-03 | 未实现 | 视频源/选集/画质/播放器集成 |
| SPEECH-01 | 未实现 | 系统/HTTP TTS 配置、断句及暂停恢复流程 |
| TOOLS-01 | 未实现 | 选词词典/查询配置和阅读交互 |
| TOOLS-02 | 部分实现：用户展示替换资源已存在 | TXT 目录规则、规则订阅/深链等其余工具导入能力 |
| BACKUP-01 | 部分实现：新 JSON/资源格式 ZIP 备份恢复和安全单元测试通过 | WebDAV 同步、平台文件选择器完整流程和真实设备恢复验证 |
| SETTINGS-01 | 部分实现：设置 JSON 与阅读样式控件存在 | 所有全局/阅读设置持久、重启和跨尺寸验收 |
| SETTINGS-02 | 未实现：当前页面未覆盖网络/缓存/Web 服务等全设置域 | 逐项实现并验证其实际生效与可回读性 |
| SYSTEM-01 | 部分实现：Rust 任务 JSON、下载/刷新任务服务命令存在 | 后台调度、完整任务 UI、取消/重试/断网恢复和平台验收 |
| SYSTEM-02 | 未实现：没有完整封面管理产品流程 | 封面来源/替换、资源更新及显示验收 |
| SYSTEM-03 | 部分实现：基础关于/帮助页面入口 | 日志导出、崩溃记录、隐私处理和升级能力 |
| PLATFORM-01 | 部分实现：桌面 browser harness 可运行；Android/iOS 架构和资源策略在接入；KMP JVM 编译成功 | Android AAR 与真机、Windows/macOS/Linux 包、iOS 包和每平台关键流程验收；当前未声称跨平台通过 |
| PLATFORM-02 | 部分实现：应用内部 loopback JSON/HTML 资源服务器存在 | 对外 Web 服务、深链、文件关联及完整授权/路由 |

| ID | 功能域 | 旧版源码依据（功能/UI 参考） | `legado_rs` 当前状态 | 完成验收要点 |
|---|---|---|---|---|
| APP-01 | 首次启动、主导航、可配置首页分组/标签/展示项 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/MainRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/main/home/HomeScreen.kt`; `HomeTabManageDialog.kt`, `HomeSectionManageDialog.kt` | 未实现；只有书源测试页 | Android、桌面、iOS 可启动进入完整主界面；首页配置保存并重启恢复；各窗口尺寸下无空白/溢出 |
| LIB-01 | 书架列表/网格、分组、排序、筛选、搜索、多选和批量管理 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/main/home/HomeScreen.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/manage/BookshelfManageScreen.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/group/` | **部分实现（2026-10-03）**：Rust shelf JSON、书籍/空分组注册表、排序与分组操作已有实现及定向测试；Vue 有书架页面。批量管理、空组端到端消费和跨平台流程仍待验收 | 添加、排序、分组、批量操作/删除后 JSON 正确更新；通知后 UI 重读资源；重启保持一致 |
| DISC-01 | 书源发现页、来源列表与发现分类、订阅分类、置顶/收藏 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/main/explore/ExploreScreen.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ExploreShowRoute.kt` | **部分实现（2026-10-03）**：Rust 私有分类 URL 映射、opaque category ID、公开 metadata/card 资源和 KMP operation adapter 已落盘；Kotlin/JVM 编译通过，真实引擎 fixture、发现 UI、分页、置顶收藏未验收 | 展示结果来自 Rust/KMP 处理后的资源；分页/筛选/刷新/收藏可用；源定义不由前端执行或当内容资源消费 |
| SEARCH-01 | 单源和多源搜索、分页/筛选/结果合并、搜索历史、无结果与错误恢复 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/SearchRoute.kt`; `SearchContentRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/search/`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/SearchKeyword.kt` | **部分实现（2026-10-03）**：Rust/KMP 搜索资源和真实浏览器搜索流程已运行；浏览器加入书架步骤被详情覆盖层挡住，正在修复。筛选、历史及完整错误恢复未验收 | 真实多源搜索、翻页、取消/失败/部分成功、结果继续打开详情；结果经 Rust 更新资源/发轻量通知，JS 读取并展示 |
| BOOK-01 | 书籍详情、元信息刷新、加入/移出书架、书籍信息编辑 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/BookInfoRoute.kt`; `BookInfoEditRoute.kt`; `app/src/main/java/io/legado/app/model/webBook/BookInfoRefresherImpl.kt` | 部分实现：有 `bookInfo` 引擎测试操作，无书目持久化或完整交互 | 详情展示处理后 JSON；加入书架时 Rust 创建/初始化书籍 JSON；编辑/刷新后跨页与重启状态一致 |
| READ-01 | 在线章节目录、目录搜索/定位、目录刷新与换源/分卷章节源 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/TocRoute.kt`; `ChangeSourceRoute.kt`; `ChangeChapterSourceRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/toc/` | 部分实现：有 `chapters` 引擎测试操作，目录不持久化为应用资源 | Rust/KMP 取得与整理目录 JSON；JS 直接消费目录、搜索/定位/切换可用章节；换源后章节地址和进度语义正确 |
| READ-02 | 正文取得、章节 HTML 缓存、图片/认证资源 URL、缓存前后补充 | `app/src/main/java/io/legado/app/model/ReadBook.kt`; `CacheBook.kt`; `service/CacheBookService.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ReaderRoute.kt` | 部分实现：`content` 可返回文本给 demo IPC；还没有 HTML 缓存/JSON `src` 契约，当前展示链不符合目标架构 | KMP 处理书源正文和替换；Rust 写含默认样式的章节 HTML 和章节 `src`；图片/认证资源在 Android、桌面、iOS 都可由 WebView 读取；缓存不足时补章，缓存足够时翻章不调 Rust |
| READ-03 | 阅读器：排版、分页/滚动、主题、手势、键盘/音量翻页、窗口自适应 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ReaderRoute.kt`; `ReadLayoutConfigDialog.kt`; `ReadStyleDialog.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/read/` | 未实现 | JS/WebView 直接消费 HTML；滑动/滚动、分屏/旋转/窗口变化、字号行距背景变更时展示正确；展示样式覆盖不回写缓存 HTML |
| READ-04 | 阅读进度低频持久化、离书/后台保存、重启恢复 | `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/BookProgress.kt`; `Book.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ReaderRoute.kt` | 未实现 | JS 记录阅读交互，按生命周期委托 Rust 更新 JSON；正常离开/后台、强杀后的实际可保证范围和重启恢复均有明确测试；强杀不可依赖未保证的退出钩子 |
| READ-05 | 阅读设置与显示层替换/净化规则 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ReplaceRuleRoute.kt`; `ReplaceEditRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ReadStyleDialog.kt`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/ReplaceRule.kt` | **部分实现（2026-10-03）**：Rust canonical `replacement-rules.json` 服务与 Vue 编辑/文本节点展示替换已有代码；旧 settings 字段弃用清空及 HTML 缓存只读的实际 UI 验收待验证 | Rust JSON 设置持久化；JS 读取 HTML 后应用用户字号/样式和展示层替换，调整不改缓存；书源规则替换只在引擎阶段做，缓存值正确 |
| READ-06 | 书签、阅读历史/记录、统计 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/BookmarkRoute.kt`; `ReadRecordRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/about/ReadRecordScreen.kt`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/Bookmark.kt`, `ReadRecord.kt` | **部分实现（2026-10-03）**：Rust JSON CRUD、session 幂等、书籍/本地日汇总和重启测试已实现；Vue 有书签/阅读洞察入口。实际阅读触发保存、完整统计呈现和跨平台流程待验收 | 书签可新增、编辑、跳转、删除；阅读记录按时间/书籍展示；Rust 更新 JSON 后可恢复，记录统计不重复/错算 |
| SOURCE-01 | 书源管理：添加、编辑、导入/导出、启停、分组、排序、校验、删除 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/BookSourceManageRoute.kt`; `BookSourceEditRoute.kt`; `app/src/main/java/io/legado/app/ui/book/source/manage/BookSourceViewModel.kt` | 部分实现：前端可输入书源 JSON 并调用引擎，但没有 Rust 管理的持久化/UI 工作流；只支持 operation 调试 | 兼容现有引擎规则与 JSON；配置文件由 Rust 管理；输入规则由 KMP 执行，WebView 不解释或运行规则；批量操作、导入校验和格式错误可恢复 |
| SOURCE-02 | 书源调试：搜索、详情、目录、正文及单步规则检查 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/BookSourceDebugRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/SourceToolboxRoute.kt`; `app/src/main/java/io/legado/app/model/CheckSource.kt` | 部分实现：现有 demo 覆盖搜索、详情、目录、正文；未证明产品 UI、单步调试或平台完整性 | 诊断请求经 Rust/KMP；结果仅为处理后数据/资源；网络失败、规则解析失败、日志敏感信息遮蔽、取消均可验证 |
| SOURCE-03 | 书源登录、Cookie/请求头/代理/重定向及来源变量持久化 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/root/SourceLoginOverlayDialog.kt`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/Cookie.kt`; `app/src/main/java/io/legado/app/model/analyzeRule/AnalyzeUrl.kt`; `legado_rs/src-tauri/src/source_http.rs`, `source_storage.rs` | 部分实现：Rust Host 具备 HTTP/键值/ Cookie 类引擎存储底座，非用户级书源管理 | 登录来源请求、Cookie 隔离/重用/清理、重定向、代理认证可跨 Android/桌面/iOS 验证；规则不泄露给前端消费 |
| SOURCE-04 | 规则帮助、源过滤规则、失效检查、JS 扩展/QuickJS 兼容 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/SourceFilterRuleRoute.kt`; `SourceToolboxRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/widget/dialog/HelpDialog.kt`; `kotlin/kmp-engine/` | 部分实现：KMP/QuickJS 引擎存在；Rust 端 README 明确 `image.*` 图片像素处理尚未接入；应用管理界面不存在 | 支持清单按规则级验收；失效检查给出可解释报告；QuickJS 兼容功能按实测标注；暂缺的图片切片/解密等能力补上或明确为唯一开放缺口，不宣称全书源兼容 |
| IMPORT-01 | 本地 TXT/EPUB/PDF/压缩文件导入、目录识别、扫描与授权 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ImportBookRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/import/local/ImportBookScreen.kt`; `app/src/main/java/io/legado/app/ui/book/import/local/ImportBook.kt`; `app/src/main/java/io/legado/app/model/fileBook/` | 未实现 | Android、桌面、iOS 的选择/授权/读取与错误提示实测；编码、章节识别、压缩包及 EPUB 资源可用；导入书籍纳入新的 JSON 模型，不要求旧备份兼容 |
| IMPORT-02 | 远程书籍、WebDAV/远程服务、上传下载与服务端导入 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/RemoteBookRoute.kt`; `data/src/jvmAndAndroidMain/kotlin/io/legado/app/model/remote/RemoteBookWebDav.kt`; `app/src/main/java/io/legado/app/service/WebService.kt` | 未实现 | 服务器配置与凭据安全保存；列出/下载/上传/导入资源；失败重试及取消工作；各平台网络策略可用 |
| RSS-01 | RSS/订阅源管理、订阅内容列表与阅读 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ReadRssRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/rss/`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/OldRssSource.kt` | **部分实现（2026-10-03）**：旧 RSS 通过已有 KMP adapter；无规则 Atom/RSS 通过 `feed-rs`，Rust 定向 fixture 验证了规则隔离、HTML sanitizer、稳定 `resource://` 引用和重启读取。RSS UI、已读状态及目标平台流程未验收 | 订阅内容通过 Rust/KMP 更新为 JSON/HTML 资源；前端阅读已处理资源；已读、筛选与刷新状态可持久化 |
| MEDIA-01 | 漫画阅读、图像源、缩放、翻页与漫画显示设置 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/MangaReaderRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/manga/`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/rule/ContentRule.kt` | 未实现 | 图片 `src` 在各目标平台可用；长图/多图、缩放、翻页和内存压力验证；需要图像解密/切片/旋转的书源功能由规则引擎/资源处理路径完成 |
| MEDIA-02 | 在线音频书/章节、播放控制、播放列表、锁屏/后台与歌词 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/AudioPlayRoute.kt`; `app/src/main/java/io/legado/app/service/AudioPlayService.kt`; `AudioPlayer` related `data` media models | 未实现 | 音频 URL/鉴权、播放暂停/跳转、前后台和平台媒体控件验证；节目/播放状态 JSON 保存恢复 |
| MEDIA-03 | 视频来源、选集、清晰度、播放控制与全屏 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/VideoPlayRoute.kt`; `app/src/main/java/io/legado/app/service/` media services | 未实现 | 引擎产出的可播放资源/清晰度列表由 JS 展示，平台播放器完整播放与错误路径通过目标设备验证 |
| SPEECH-01 | 系统朗读、HTTP TTS、语音配置、断句/跟读控制 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ReadAloudConfigDialog.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/OtherConfigRoute.kt`; `app/src/main/java/io/legado/app/model/ReadAloud.kt`; `app/src/main/java/io/legado/app/service/TTSReadAloudService.kt`, `HttpReadAloudService.kt` | 未实现 | 当前页/章节可从 HTML 朗读；系统与 HTTP 引擎分别验证、可暂停恢复、切章、后台行为符合各平台能力；HTTP TTS 配置由 Rust 持久化 |
| TOOLS-01 | 词典规则、选词查词、文本/图片菜单交互 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/DictRuleRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/dict/`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/DictRule.kt` | 未实现 | 选择词语能调用已配置字典/查询服务；阅读页与 PDF/图片适用能力有明确行为；规则及配置 JSON 可管理 |
| TOOLS-02 | TXT 目录规则、替换规则、规则订阅/导入清单与深链导入 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/TxtTocRuleRoute.kt`; `ReplaceRuleRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/association/`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/RuleSub.kt` | 未实现 | 文件/深链/链接导入正确路由到新 JSON 模型；变更由 Rust 验证、落盘；界面不执行书源定义 |
| BACKUP-01 | 备份、恢复、WebDAV 同步和冲突/失败处理 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/BackupConfigRoute.kt`; `app/src/main/java/io/legado/app/help/storage/Backup.kt`, `Restore.kt`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/Server.kt` | 未实现；明确不兼容旧版备份格式 | 新格式能备份并恢复新项目全部适用 JSON 和资源；远程同步可认证/重试；中断不损坏本地数据；拒绝旧格式时给出清楚错误 |
| SETTINGS-01 | 全局和阅读设置：主题、阅读布局、字体、配色、页面按键及首页导航 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ThemeConfigRoute.kt`; `WelcomeConfigRoute.kt`; `ReadLayoutConfigDialog.kt`; `ReadStyleDialog.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/compose/preference/` | 未实现 | 设置作为 JSON；跨设备尺寸和亮/暗主题检查；HTML 默认值可被用户设置覆盖但缓存不变；退出/重启恢复 |
| SETTINGS-02 | 其它设置：语言、网络/UA、缓存、预下载、字体、Web 服务、快捷键等 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/OtherConfigRoute.kt`; `MyConfigScreen.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/root/PlatformCapabilityProviders.kt` | 未实现 | 仅呈现当前平台实际支持的设置；每项配置会生效且有可回读状态；系统能力差异不显示无效开关 |
| SYSTEM-01 | 任务与下载：书籍更新、预下载、全量任务、进度、取消和错误恢复 | `app/src/main/java/io/legado/app/service/UpdateBookService.kt`; `CacheBookService.kt`; `DownloadService.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/OtherConfigRoute.kt` | 未实现；无任务状态 JSON 或任务界面 | Rust 管理任务与状态 JSON；后台/前台任务生命周期、取消、断网、部分成功、重复启动和重试均验收；JS 读取资源展示状态 |
| SYSTEM-02 | 封面/主题图片管理、封面来源或替换 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/CoverConfigRoute.kt`; `changecover/ChangeCoverDialog.kt`; `app/src/main/java/io/legado/app/model/BookCover.kt` | 未实现 | 封面 `src` 稳定消费；本地图片更新后书架一致；主题/默认封面适配分辨率和平台 |
| SYSTEM-03 | 关于/帮助/日志/更新/崩溃记录 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/AboutRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/about/AboutScreen.kt`; `CrashLogsDialog.kt`; `HelpDialog.kt` | 未实现 | 用户可访问帮助、查看版本/诊断信息、导出日志；信息不泄露书源敏感配置；升级能力按平台可交付 |
| PLATFORM-01 | Android、Windows/macOS/Linux、iOS 一致产品流程、文件/剪贴板/窗口/外链等平台适配 | `legado/README.md` 平台声明; `app/src/main/java/io/legado/app/ui/main/AndroidPlatformServices.kt`; `desktop-core/`; `ui/src/iosMain/` | 部分实现：Tauri 配置含桌面、Android、iOS；平台只有书源引擎桥接，没有完整产品 UI/服务 | 在 Android 真机、桌面系统和 iOS 模拟器/设备完成各自打包启动；主流程和资源 URL 实测；平台不支持的系统行为明确降级且不显示虚假控件 |
| PLATFORM-02 | 本地 HTTP/Web API、外部链接/文件关联及导入入口 | `legado/README.md` API; `app/src/main/java/io/legado/app/api/`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/WebViewRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/association/` | 未实现 | 接口从 Rust 提供并校验权限；Web/深链/API 操作最终更新 JSON 并通知 UI；非法输入和路径权限测试 |

## `legado_rs` 基线

当前实现集中在根级 Tauri+Vue 起始页和书源引擎宿主：

- `src/App.vue` 是单页“书源测试”工作台，允许输入书源 JSON，并按搜索、详情、目录、正文调用 Rust；它不是完整书架、发现或阅读产品 UI。
- `src/api/sourceEngine.ts` 和 `src-tauri/src/lib.rs` 连接 `execute_source_engine` 命令；`src-tauri/src/source_engine.rs` 将操作交给 KMP/JNI；Tauri 插件提供 Android 与 iOS 的引擎入口。
- 现有引擎操作类型为 `search`、`bookInfo`、`chapters`、`content`。Rust 同时承载引擎 HTTP/存储宿主适配；`source_storage.rs` 保存引擎命名空间键值数据。
- `src-tauri/src/source_storage.rs`、`source_engine.rs` 存在少量异步 Rust 测试代码；这不是整 app 的端到端验证，当前也没有产品流程/跨平台验收记录。
- 目前未发现书架/进度/设置/任务 JSON 模型、章节 HTML 资源缓存、资源 URL 契约、产品路由、阅读器或完整功能服务。

## 旧版成熟度边界

以下证据用于把“新项目缺功能”和“旧版本身有缺口/平台限制”区分开：

| 旧版证据 | 可以确认的限制 | 新项目处理 |
|---|---|---|
| `legado/README.md` 的平台表 | README 明确称 Android 为主力且功能完整；桌面端、iOS 和鸿蒙仍在开发/理论支持状态。共享 Kotlin/UI 文件或 route 存在，不等于其它平台真实可用。 | 目标平台是 Android、桌面、iOS；每个都必须独立跑主要端到端流程，鸿蒙不做。不得把旧版 Android 测试结果当作其它平台验收。 |
| `legado/ui/src/ohosMain/kotlin/io/legado/app/ui/browser/OhosWebViewStub.kt` | 源码明确是占位 actual，脚本/浏览器宿主桥接未完成。 | 鸿蒙虽非当前目标，仍不复制这类占位 UI/无操作实现到新平台。 |
| `legado/desktop-core/src/main/kotlin/io/legado/desktop/model/fileBook/DesktopFileBookAccessor.kt` | 该模块中 PDF 读取需注入 PDFBox handler，headless path 不支持；压缩包处理需注入 archive support，headless path 不支持。不能据此推断带 desktop 依赖的最终桌面 app 也一定不支持，但必须检查实际打包路径。 | 本地文件功能以本项目最终平台包实测；PDF/压缩包不可用时继续补齐，而不是照搬 headless 限制。 |
| `legado_rs/README.md` 的现有能力说明 | 当前 KMP 引擎接入明确写明尚未接入 JS `image.*` 的图片像素处理；依赖图片解密、切片、拼接或旋转的规则暂不可用。 | 这是新项目已知的引擎/宿主缺口，纳入 SOURCE-04 验收；完成前不能宣称现有格式/规则能力整体完整。 |
| 旧版 `legado/data/src/roomEntitiesMain/...` 实体及 Room/AppDatabase；对照新项目 `src-tauri/src/source_storage.rs` | 旧版大量业务实体由 Room/数据库承载；新项目现有 JSON namespace 只是 KMP 引擎缓存适配，不是新产品领域存储，也不表示已实现书架/设置 JSON。 | 业务模型须按本项目 JSON/HTML/`src` 架构新建；不迁移旧数据库/备份，也不引入 SQLite。 |

未在源码中找到明确占位证据的旧版功能，不应被武断标为“旧版未实现”；先按旧源码列出的成熟入口和模型做行为核查，再设计满足本项目资源架构的实现。旧版已知不完整的功能也不能为了 UI 相似度原样照搬，需按 D-001 和 [`architecture.md`](architecture.md) 补齐或在决策/验证记录中说明真实阻塞。
