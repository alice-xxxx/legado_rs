# 功能验证记录

本文件记录真实可执行的验证和证据。每项功能先列端到端场景，再记录实际平台、设备/运行模式、命令或操作、结果及未覆盖部分。没有实际结果时保持“待验证”，不得将源码盘点或编译结果写成完整功能可用。

## 状态约定

- **待实现**：功能尚无可验证实现。
- **待验证**：已有实现，尚无记录证明流程可用。
- **通过**：记录了实际操作和预期结果，且关键流程无未解决故障。
- **失败/阻塞**：失败可复现；同时写诊断、下一步和独立可继续工作。
- **部分通过**：只验证了列明的部分；不得提升对应 feature-matrix 行为“已验收”。

## 当前静态盘点

| 日期 | 检查范围 | 方法 | 结果 | 限制 |
|---|---|---|---|---|
| 2026-10-02 | `legado_rs` 现有入口、Tauri command、source-engine 操作/宿主、依赖清单；旧项目 UI route、服务和主要数据模型 | 只读源码与文件清单检查；未改应用文件，未运行旧项目构建 | 确认新项目目前以书源测试页及 KMP 书源引擎宿主为主；确定功能缺口见 [`feature-matrix.md`](feature-matrix.md) | 这是静态盘点，不是应用功能验证；不将任何 app 功能记为通过 |

## 2026-10-03 开发中验证

以下仅记录已实际运行的范围。Rust 模块测试与 Kotlin 源码编译不能替代完整 Vue 用户流程、真实桥接运行时或平台包验收。

| Feature ID | 命令或操作 / 环境 | 实际结果 | 未覆盖 / 后续动作 |
|---|---|---|---|
| Rust headless library | `/workspace/legado_rs/src-tauri`: `source /workspace/.setup/activate.sh && cargo check --no-default-features --lib` | **通过**；lib 编译成功，仅有 `source_jni` 宏触发的弃用警告 | 不包含 Tauri 桌面/移动 app binary；需由各平台目标构建确认实际包 |
| Rust formatting | `/workspace/legado_rs/src-tauri`: `source /workspace/.setup/activate.sh && cargo fmt --check` | **通过** | Formatting is not functional verification |
| DISC-01 / RSS-01 | `/workspace/legado_rs/kotlin`: `source /workspace/.setup/activate.sh && ./gradlew :kmp-engine:compileKotlinJvm --no-daemon --no-configuration-cache`（Linux/JVM） | **源码编译通过**；新增 `exploreKinds`、`explore`、`rssExploreKinds`、`rssExplore`、旧 RSS book/chapter/content adapter 均参与编译 | 这是 JVM 源码编译，不是 engine fixture 运行；重新准备 desktop runtime/Android AAR 后仍需真实分类/分页用例。Linux 未编译 iOS native targets |
| RSS-01 | `/workspace/legado_rs/src-tauri`: `cargo test --no-default-features --lib standard_atom_subscription_publishes_processed_cards_and_html_resources -- --nocapture` | **Rust 测试通过（1/1）**。导入无规则 Atom URL、生成 opaque 分类、解析文章、写 sanitizer HTML、检查规则和正文不在公开/私有 JSON、确认磁盘使用稳定 `resource://`，重启服务后重读列表并获取 HTML | 覆盖普通 feed parser/backend；不覆盖旧 RSS 的 KMP 运行时、RSS UI、收藏/已读状态、移动/桌面平台网络策略 |
| LIB-01 | `/workspace/legado_rs/src-tauri`: `cargo test --no-default-features --lib shelf_groups_and_sort_modify_the_shared_shelf_json -- --nocapture` | **Rust 测试通过（1/1）**。书架排序、每书分组、顶层空分组 registry、重命名/删除和 ResourceStore reopen 后保留空分组 | Vue 空组管理、并发的进度更新/预取整合和跨平台 UI 流程仍待验收 |
| 全套 Rust 测试（修复前记录） | `/workspace/.setup/resumed-core-tests.log` | **历史运行：21/22**；唯一失败为 EPUB 相对 media path assertion，随后修复。仅保留作诊断轨迹 | 最新结果见下一行；还需实际 Vue 阅读生命周期、统计展示、书签导航验证 |
| 全部已实现 Rust 单元/集成覆盖（headless） | 2026-10-03 `/workspace/legado_rs/src-tauri`: `source /workspace/.setup/activate.sh && cargo test --no-default-features --lib` | **通过：34/34，0 失败**。覆盖 backup、discovery projection/ID、TXT/EPUB、书签与阅读历史日统计、分组/排序、资源安全/HTML/media proxy、标准 Atom RSS fixture、Rust→KMP search/detail/catalog/content fixture、稳定资源服务。此次 `--no-default-features` 不依赖 GTK/WebKitGTK | Rust headless 验证不代表 Vue E2E 或桌面/Android/iOS app 已验收。此前 21/22 的运行记录见 `/workspace/.setup/resumed-core-tests.log`；其 EPUB 断言随后修复并在本次全套运行通过 |
| Rust locked headless 重跑 | `cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j4` | **通过：34/34**；application_core 独立复核 lockfile 下结果 | 未覆盖任务暂停/恢复/取消、备份恢复并发，以及 Vue 和各平台用户流程 |
| Tauri desktop 类型检查 | desktop feature check；构建期间设置 `TAURI_CONFIG='{"bundle":{"resources":[]}}'` | **通过**（application_core 的集成验证）；跳过构建脚本在该环境打包 runtime resources 时遇到的 Permission denied 路径 | 类型检查不是桌面应用 bundle/启动验证；仍需真实打包与 WebView 资源读取 |
| SEARCH-01 | `/workspace/legado_rs`: 浏览器自动化 `/workspace/.setup/browser-e2e-run-2026-10-03.log` | 搜索操作与搜索结果由真实浏览器运行；随后点击“加入书架”失败，详情覆盖层遮挡了目标按钮。不得据此标记搜索域验收 | 修正覆盖层命中后重跑完整搜索→详情→加入书架流程；分页/历史/错误恢复待测 |

## 端到端用例目录

为每个平台分别记录运行环境和结果。若一个流程部分共用实现，也要分别确认 WebView 的资源 URL、权限和后台行为。

| 流程 | 覆盖步骤 | Android | Windows/Linux/macOS | iOS |
|---|---|---|---|---|
| 搜索并开始阅读 | 搜索 → 详情 → 加入书架 → 目录 → 阅读；KMP 执行规则，Rust 产生 JSON/HTML/`src`，JS 读取展示 | 待实现 | 待实现 | 待实现 |
| 章节缓存边界 | 打开书 → 检查缓存 → Rust 补章 → Rust 更新目录/资源 → JS 切换本地已缓存章时不重新取内容 | 待实现 | 待实现 | 待实现 |
| HTML 显示覆盖 | 缓存默认样式 → 改阅读字号/行距/颜色和显示层替换 → 退出/重开仍用原 HTML，当前设置生效 | 待实现 | 待实现 | 待实现 |
| 进度保存恢复 | 连续阅读 → 离书或进后台 → Rust 更新 JSON → 结束/重启 → 恢复正确章节和位置 | 待实现 | 待实现 | 待实现 |
| 书架 JSON 修改 | 加书 → 分组/排序 → 删除 → Rust 改文件并通知 → JS 重新读资源；重启后保持一致 | 待实现 | 待实现 | 待实现 |
| 书源兼容与隔离 | 导入当前引擎支持的书源 → Rust 管理并交 KMP 执行 → 验证规则未由 WebView 执行、前端只接收处理结果 | 待实现 | 待实现 | 待实现 |
| 离线及失败恢复 | 缓存章节离线读取；无缓存网络失败、取消、重试、URL 失效、文件写入错误能恢复并说明状态 | 待实现 | 待实现 | 待实现 |
| 多媒体和本地文件 | 按 feature matrix 对 RSS/漫画/音频/视频/本地 TXT/EPUB/PDF 的适用链路逐项验证 | 待实现 | 待实现 | 待实现 |

## 实际运行记录模板

```text
日期 / 提交：
Feature ID：
目标平台、OS 版本、设备/模拟器/浏览器窗口：
前置数据与权限：
执行命令或手动步骤：
预期：
实际：
结果：待验证 / 通过 / 部分通过 / 失败 / 阻塞
证据路径（日志、截图、输出）：
未覆盖及后续动作：
```

## 2026-10-03 核心实现验证

| 流程 | 平台 / 环境 | 命令或操作 | 实际结果 | 限制及后续动作 |
|---|---|---|---|---|
| 本地 TXT / EPUB 导入和资源读取 | Linux headless Rust integration tests；独立 target `/tmp/legado-rs-core-isolation-target`；unset GTK 相关环境变量 | `source /workspace/.setup/activate.sh`; `cargo test --locked --no-default-features --lib local_books::tests -- --nocapture` | 4/4 通过：UTF-8 BOM 多章 TXT、GBK/Unicode 大文本无目录回退、重复导入保留进度；EPUB 按 OPF spine 顺序解析，经过净化的章节 HTML、CSS、本地 PNG 和 CSS 引用 PNG 均从实际 loopback HTTP 服务取回。 | 验证了 Rust 解析、JSON/HTML/asset 持久化和 HTTP 资源消费；原生文件选择器及 Android/桌面/iOS 的 WebView 用户流程仍待平台验证。 |
| 完整资源备份 / 恢复 | Linux headless Rust integration tests；同一独立 target | `source /workspace/.setup/activate.sh`; `cargo test --locked --no-default-features --lib backup::tests -- --nocapture` | 3/3 通过：恢复已删除书籍/进度和章节图片，删除备份之后创建的书；恢复任务 cursor、搜索/发现结果、来源私有映射和 KMP 用户态 cookie/变量；保留设备当前 source cache；不安全 ZIP 路径不改现存数据；模拟两次目录 rename 边界后启动恢复。恢复后运行中的 ResourceServer 经章节相对图片 URL 实际取回有效 1×1 PNG。 | Tauri picker 命令、UI 提示、Android/桌面/iOS 备份文件选择和 WebView 刷新仍待集成验收；ZIP 未加密，可能包含书源与会话数据。 |
| 无界面依赖边界 | Linux Cargo feature graph | `cargo tree --locked --no-default-features --edges normal` 和 `cargo tree --locked --no-default-features --edges build`，输出过滤 GTK/WebKit/Wry/Tauri 插件 | 两条依赖图均无匹配项；纯逻辑测试不编译 Tauri 构建插件或桌面 WebView 依赖。 | 真实默认桌面和移动运行时仍须在各自平台构建验证；这项检查不代替完整 app 构建。 |

以上均为核心实现级验证，不代表完整 app 或三平台用户流程已验收。
