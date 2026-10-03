# 功能盘点与验收矩阵

本清单的旧版功能与源码依据最初于 **2026-10-02** 盘点；旧版源码参照及验收标准仍保留在下方矩阵。它描述需要在 `legado_rs` 中实现并实际验收的能力，不表示旧项目每个旧行为都正确，也不要求保留旧版 bug 或未完成 UI。源码路径相对于对应仓库根目录。`旧版源码依据` 表明需求和原有实现线索，**不单凭 route/页面存在推定旧版各平台功能成熟**；源码可确认的旧版缺失/未完备项另列在“旧版成熟度边界”。下方矩阵中的 `legado_rs` 状态记录了首轮盘点和部分后续历史快照（有日期处以日期为准），不再作为当前状态；当前状态以其前的 2026-10-03 覆盖表为准。

状态含义：**未实现** = 当前没有产品级实现；**部分实现** = 有产品代码或经过验证的纵向切片，但功能范围或目标平台验收未完整；**已验收** = 真实用户流程已经可用并有证据。下表是 **2026-10-03 当前状态覆盖表**，优先于首轮快照矩阵的状态单元格。最近一次完整 locked headless suite 是 Stage 2 快照的 154/154（0 failed/ignored/filtered，4.53 秒；日志 `/tmp/legado-stage2-full-tests.log`），包含尚属 draft 的 source-browser-host 测试，不代表该功能已实现。此前 119/119、144/144 属于较早快照。`refresh_book_info` 已在 Stage 2 之后加入 Rust service，但当前没有针对该方法的新测试，也没有 Tauri/UI 接入或用户流程验收。Chromium 41 IPC 阅读/缓存/目录刷新记录见 `validation.md` 中 `2026-10-03-cache-catalog-e2e-38913f5`，整书换源 Chromium 用户流程见“整书换源浏览器端到端”，Linux 原生 AppImage TXT 流程则属于 a9168dd 快照。没有一次编译或单元测试结果代表完整 app 或跨平台功能验收。

## 当前实现与缺口（2026-10-03）

| ID | 当前状态与证据 | 本次提交快照仍缺少的工作 |
|---|---|---|
| CORE-TEST | 部分实现：Stage 2 快照的最新完整 locked headless suite 为 154/154（0 failed/ignored/filtered，4.53 秒）；suite 含 source-browser-host draft tests，不能据此标记其产品功能完成。另有 38913f5 阅读、缓存、目录刷新 Chromium 流程的 41 次 Rust IPC | Stage 2 之后新增的 `refresh_book_info` 尚无 focused test；Rust 测试结果不代替 Android、桌面安装包或 iOS 用户流程 |
| APP-01 | 部分实现：主页配置器支持 tab 新建/改名/删除/排序，以及 section 新建/改名/删除/排序、来源/分类和展示样式配置；当前 Chromium E2E 验证配置保存及 Rust service 重启后的恢复 | 配置器的异常来源/保存失败恢复和各平台窗口流程仍待验收；发现页快捷添加仍放在第一个 tab |
| LIB-01 | 部分实现：书架 JSON、排序、每书分组和空分组注册表；Vue 可筛选、搜索、改分组与排序；focused Rust 测试覆盖持久化 | 批量选择/批量管理全流程和多平台书架验收 |
| DISC-01 | 部分实现：KMP 分类/分页通过 Rust adapter；分类地址保存在 private map，公开资源只含处理后 metadata/card；收藏 JSON、Tauri 命令和 Vue 收藏列表/开关已接入；KMP 第 1–3 页 fixture 通过；当前工作树 Chromium E2E 验证收藏分类并加入主页 | 已验证的浏览器 fixture 不代替错误/刷新恢复及各平台验证；真实来源、收藏列表更新与完整发现产品边界仍需逐项验收 |
| SEARCH-01 | 部分实现：多源选择、异步搜索任务、分页、部分错误呈现和任务取消接口/UI 已有；SearchHistory 的最近/常用列表、复用、删除/清空和重启持久化已由当前 Chromium E2E 验证；Rust search-history helper tests 5/5 通过 | 重复搜索后按钮忙碌状态有已记录的 UI 竞态；结果筛选/排序、部分失败/取消边界和各平台搜索流程仍待验收 |
| BOOK-01 | 部分实现：新书加入、书籍资源读取和整书换源已将处理后的 `bookInfo` 投影到 `BookDocument` 的 intro/kind/word count/source labels；Rust 保留 canonical 来源与原始引擎书籍数据在 private data。Projection tests 5/5、metadata backup/restore roundtrip 在 `backup::tests` 4/4 通过；Stage 2 后已新增调用 `bookInfo`/`rssBookInfo` 的 Rust `refresh_book_info` service，尚未有该方法的 focused test 或 Tauri/UI 接入 | 搜索结果卡片仍走独立字段投影，尚未共用 intro/cover 的清理 helper；完整详情 UI 展示、刷新并发/部分响应边界和各平台流程待验收 |
| READ-01 | 部分实现：目录以 JSON 资源消费；Vue 有本地目录搜索/定位；目录刷新/新章节检查及身份/进度重排由 Rust 处理，41 IPC 浏览器流程验证了目录重排后进度恢复。当前工作树的整书换源 Chromium E2E 已通过：搜索候选绑定书籍与目录快照，提交前复核来源/目录，按新 generation 隔离章节缓存，并按唯一章节标题迁移进度与书签 | 设备/平台流程和其他来源变更、删除及提交失败边界仍需验证。单章换源尚未实现；离线目录及各平台流程待验收 |
| READ-02 | 部分实现：正文由引擎处理后存为 HTML，书籍 JSON 用稳定章节 `src`；JS 读取资源并负责翻页/展示；41 IPC Chromium 流程覆盖单章缓存、后台预取失败后重试及目录变更 | 图片认证资源、多平台 WebView 与离线完整边界尚未验收；后台预取/缓存细节需结合设备验证 |
| READ-03 | 部分实现：Vue 阅读器支持章节导航、列分页、样式覆盖和键盘/按钮交互；浏览器覆盖 36 段分页及样式变化不写回 HTML；旧 Linux AppImage 曾验证 TXT 阅读和重启恢复 | 手势、旋转/窗口变化、大文本与 Android/iOS 阅读器流程未全量验收；旧 AppImage 结果不能代表当前提交 |
| READ-04 | 部分实现：Rust 保存 progress JSON；Vue 离开/隐藏阅读页时低频保存；41 IPC 覆盖目录重排后的进度映射与重开恢复 | 后台挂起/强杀边界及 Android、iOS 真机生命周期未验证；不依赖无法保证的强杀退出钩子 |
| READ-05 | 部分实现：canonical display replacement JSON、设置 UI 与 JS 展示层规则已存在 | 替换 scope、无副作用 HTML/text-node-only 及重启流程未做完整 UI E2E；书源替换仍由 KMP 执行 |
| READ-06 | 部分实现：书签 JSON CRUD、阅读 session/day 聚合和 Vue 阅读洞察已接线；整书换源 Chromium E2E 验证了匹配书签迁移及孤立书签保留 | 从书签跳转、旧章保留策略和长时间统计的端到端/UI 验收待补 |
| SOURCE-01 | 部分实现：Rust 私有源存储；JSON 文件导入、启停、名称/分组 metadata 编辑和删除；WebView 只收 metadata | 没有用户级规则编辑/导出/排序/批量校验/调试完整流程；逐平台源导入、管理和错误恢复待验收 |
| SOURCE-02 | 部分实现：现有引擎操作可经 Rust 调用，task/result 错误可展示 | 产品化书源调试、逐步规则诊断、帮助和敏感日志遮蔽流程未完成 |
| SOURCE-03 | 部分实现：Rust HTTP/storage/Cookie 宿主与真实 POST 重定向 fixture 存在 | 用户级登录、UA/代理/请求头配置、Cookie 交互和 Android/iOS 运行时兼容仍待验证 |
| SOURCE-04 | 部分实现：保留现有 KMP/QuickJS；Rust ImageOps、C ABI、JVM/JNI 脚本 fixture 已通过；iOS NativeJS bridge 代码/目标平台验证仍需 CI/运行证据 | 逐规则格式兼容矩阵及各平台宿主能力实测；不把 JVM/JNI 通过推断成 iOS 通过 |
| IMPORT-01 | 部分实现：Rust TXT/EPUB/CBZ/PDF 解析和 picker UI 已接入；headless 本地导入测试覆盖 8/8；当前 Chromium E2E 验证 TXT 规则 UI 持久化、实际 TXT 导入与章节资源重启恢复；Linux AppImage 曾实测 TXT 选择、阅读、进度重开 | PDF/CBZ 选择/渲染的浏览器与设备流程、权限/错误恢复及 Android/iOS 导入待验收 |
| IMPORT-02 | 未实现：WebDAV/远程书籍导入服务当前不存在 | 远程浏览、凭据、上传/下载/重试/取消和平台测试 |
| RSS-01 | 部分实现：`88d6ba6` 已接入 RSS 订阅模式 UI、已启用 RSS 来源筛选、文章/分类分页、已读/收藏筛选、HTML 展示及取消订阅；浏览有效分类时 Rust 建立默认订阅状态。Rust state helper 定向测试 4/4 通过；当前工作树 Chromium E2E 实际验证标准 Atom 文章展示、已读/收藏、read/favorites/unread 筛选、退订清理和重启后的状态 | 此 browser E2E 使用标准 Atom；旧 KMP RSS 仅过滤引擎当前返回页，普通 feed 则先对 feed 过滤再分页。旧 KMP RSS UI 和目标平台流程仍待验收 |
| MEDIA-01 | 未实现：资源层有 media proxy，不等同漫画产品流程 | 漫画源、图像目录、缩放/翻页和内存压力验证 |
| MEDIA-02 | 未实现 | TTS/音频引擎接入、后台和平台媒体控制 |
| MEDIA-03 | 未实现 | 视频源/选集/画质/播放器集成 |
| SPEECH-01 | 未实现 | 系统/HTTP TTS 配置、断句及暂停恢复流程 |
| TOOLS-01 | 未实现 | 选词词典/查询配置和阅读交互 |
| TOOLS-02 | 部分实现：用户展示替换资源已存在 | TXT 目录规则、规则订阅/深链等其余工具导入能力 |
| BACKUP-01 | 部分实现：JSON/HTML/媒体/source private map/任务等 ZIP 备份恢复后端及安全测试通过；当前 `backup::tests` 4/4 覆盖 source-revisions 单调值和删除来源 tombstone 的归档/恢复、严格 schema 与非法 ID 拒绝；Vue picker create/restore controls 已接入 | 当前没有 WebDAV/自动同步；平台 picker→备份→修改数据→恢复→资源重读完整用户流程尚未验收。`lastBackupAt` 仅 UI 会话状态 |
| SETTINGS-01 | 部分实现：设置 JSON、阅读字号/行距/字体/主题/预读、展示替换、本地书导入、任务/统计/备份入口已有 | 各设置重启持久、跨窗口尺寸和平台生命周期验收 |
| SETTINGS-02 | 未实现：当前页面未覆盖网络/缓存/Web 服务等全设置域 | 逐项实现并验证其实际生效与可回读性 |
| SYSTEM-01 | 部分实现：Rust 持久任务、搜索/下载/目录刷新、暂停/恢复/取消命令和 Vue TaskCenter 已接入 | 任务恢复、后台调度、断网重试和平台流程仍需端到端验收 |
| SYSTEM-02 | 未实现：没有完整封面管理产品流程 | 封面来源/替换、资源更新及显示验收 |
| SYSTEM-03 | 部分实现：基础关于/帮助页面入口 | 日志导出、崩溃记录、隐私处理和升级能力 |
| PLATFORM-01 | 部分实现：Linux Chromium + Rust/KMP browser harness 有 41 IPC 阅读/目录刷新验证、Home/RSS 和换源 E2E；a9168dd Linux AppImage 已实测 TXT 导入、阅读与进度重开；CI run `37127283643` 五个平台 jobs 全部成功，unsigned iOS IPA artifact（13,073,112 B）已上传 | CI 构建与 artifact 上传不等同平台运行验收；Android、Windows、macOS、Linux 与 iOS 安装启动、资源和关键用户流程仍需逐平台实测 |
| PLATFORM-02 | 部分实现：应用内部 loopback JSON/HTML 资源服务器存在 | 对外 Web 服务、深链、文件关联及完整授权/路由 |

| ID | 功能域 | 旧版源码依据（功能/UI 参考） | `legado_rs` 首轮/后续历史状态（非当前状态，当前状态见上表） | 完成验收要点 |
|---|---|---|---|---|
| APP-01 | 首次启动、主导航、可配置首页分组/标签/展示项 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/MainRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/main/home/HomeScreen.kt`; `HomeTabManageDialog.kt`, `HomeSectionManageDialog.kt` | 未实现；只有书源测试页 | Android、桌面、iOS 可启动进入完整主界面；首页配置保存并重启恢复；各窗口尺寸下无空白/溢出 |
| LIB-01 | 书架列表/网格、分组、排序、筛选、搜索、多选和批量管理 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/main/home/HomeScreen.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/manage/BookshelfManageScreen.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/group/` | **部分实现（2026-10-03）**：Rust shelf JSON、书籍/空分组注册表、排序与分组操作已有实现及定向测试；Vue 有书架页面。批量管理、空组端到端消费和跨平台流程仍待验收 | 添加、排序、分组、批量操作/删除后 JSON 正确更新；通知后 UI 重读资源；重启保持一致 |
| DISC-01 | 书源发现页、来源列表与发现分类、订阅分类、置顶/收藏 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/main/explore/ExploreScreen.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/ExploreShowRoute.kt` | **部分实现（2026-10-03）**：Rust 私有分类 URL 映射、opaque category ID、公开 metadata/card 资源、KMP adapter 和收藏 JSON service 已落盘。定向测试 5/5 通过，含真实 KMP/JNI categories 与第 1–3 页、规则隔离和收藏重启；Tauri 收藏命令、发现 UI 与跨平台用户流程未验收 | 展示结果来自 Rust/KMP 处理后的资源；分页/筛选/刷新/收藏可用；源定义不由前端执行或当内容资源消费 |
| SEARCH-01 | 单源和多源搜索、分页/筛选/结果合并、搜索历史、无结果与错误恢复 | `ui/src/sharedUiMain/kotlin/io/legado/app/ui/route/SearchRoute.kt`; `SearchContentRoute.kt`; `ui/src/sharedUiMain/kotlin/io/legado/app/ui/book/search/`; `data/src/roomEntitiesMain/kotlin/io/legado/app/data/entities/SearchKeyword.kt` | **部分实现（2026-10-03）**：当前 browser_harness E2E 同源码快照通过搜索、加入书架、章节 HTML 阅读/翻页、进度恢复与一次 404→修复→200 恢复；完整交互证据见 `validation.md`。多源、筛选/历史、取消/部分失败和各平台产品流程仍待验收 | 真实多源搜索、翻页、取消/失败/部分成功、结果继续打开详情；结果经 Rust 更新资源/发轻量通知，JS 读取并展示 |
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

## `legado_rs` 首轮基线（2026-10-02 历史快照）

以下描述仅用于保留第一次源码盘点时的历史状态，**不是当前实现说明**；当前能力与缺口见上方 2026-10-03 覆盖表：

- 当时的 `src/App.vue` 是单页“书源测试”工作台，允许输入书源 JSON，并按搜索、详情、目录、正文调用 Rust；它不是当时完整的书架、发现或阅读产品 UI。
- `src/api/sourceEngine.ts` 和 `src-tauri/src/lib.rs` 连接 `execute_source_engine` 命令；`src-tauri/src/source_engine.rs` 将操作交给 KMP/JNI；Tauri 插件提供 Android 与 iOS 的引擎入口。
- 现有引擎操作类型为 `search`、`bookInfo`、`chapters`、`content`。Rust 同时承载引擎 HTTP/存储宿主适配；`source_storage.rs` 保存引擎命名空间键值数据。
- `src-tauri/src/source_storage.rs`、`source_engine.rs` 存在少量异步 Rust 测试代码；这不是整 app 的端到端验证，当前也没有产品流程/跨平台验收记录。
- 当时未发现书架/进度/设置/任务 JSON 模型、章节 HTML 资源缓存、资源 URL 契约、产品路由、阅读器或完整功能服务。

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
