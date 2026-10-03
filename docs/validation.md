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
| DISC-01 / RSS-01 | `/workspace/legado_rs/kotlin`: `source /workspace/.setup/activate.sh && ./gradlew :kmp-engine:compileKotlinJvm --no-daemon --no-configuration-cache`（Linux/JVM） | **源码编译通过**；新增 `exploreKinds`、`explore`、`rssExploreKinds`、`rssExplore`、旧 RSS book/chapter/content adapter 均参与编译；发现分类/分页实际运行结果见本表 DISC-01 fixture | 此项本身只记录源码编译；Linux 未编译 iOS native targets |
| RSS-01 | `/workspace/legado_rs/src-tauri`: `cargo test --no-default-features --lib standard_atom_subscription_publishes_processed_cards_and_html_resources -- --nocapture` | **Rust 测试通过（1/1）**。导入无规则 Atom URL、生成 opaque 分类、解析文章、写 sanitizer HTML、检查规则和正文不在公开/私有 JSON、确认磁盘使用稳定 `resource://`，重启服务后重读列表并获取 HTML | 覆盖普通 feed parser/backend；不覆盖旧 RSS 的 KMP 运行时、RSS UI、收藏/已读状态、移动/桌面平台网络策略 |
| LIB-01 | `/workspace/legado_rs/src-tauri`: `cargo test --no-default-features --lib shelf_groups_and_sort_modify_the_shared_shelf_json -- --nocapture` | **Rust 测试通过（1/1）**。书架排序、每书分组、顶层空分组 registry、重命名/删除和 ResourceStore reopen 后保留空分组 | Vue 空组管理、并发的进度更新/预取整合和跨平台 UI 流程仍待验收 |
| 全套 Rust 测试（修复前记录） | `/workspace/.setup/resumed-core-tests.log` | **历史运行：21/22**；唯一失败为 EPUB 相对 media path assertion，随后修复。仅保留作诊断轨迹 | 最新结果见下一行；还需实际 Vue 阅读生命周期、统计展示、书签导航验证 |
| 全部已实现 Rust 单元/集成覆盖（较早快照；headless） | 2026-10-03 `/workspace/legado_rs/src-tauri`: `source /workspace/.setup/activate.sh && cargo test --no-default-features --lib` | **当次通过：34/34，0 失败**。覆盖 backup、初期 discovery projection/ID、TXT/EPUB、书签与阅读历史日统计、分组/排序、资源安全/HTML/media proxy、标准 Atom RSS fixture、Rust→KMP search/detail/catalog/content fixture、稳定资源服务。此次 `--no-default-features` 不依赖 GTK/WebKitGTK | 此为添加 discovery KMP fixture、CBZ/PDF 和资源扩展测试前的运行记录，不代表当前全套 suite 已重跑。Rust headless 验证也不代表 Vue E2E 或桌面/Android/iOS app 已验收。此前 21/22 的运行记录见 `/workspace/.setup/resumed-core-tests.log` |
| Rust locked headless 重跑（较早快照） | `cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j4` | **当次通过：34/34**；application_core 独立复核 lockfile 下结果 | 当前 suite 后续增加了 discovery/local-book/resource tests；尚未以一次全量运行重新记录总数 |
| 当前锁定 Rust headless 全套 | 2026-10-03；root 对当前快照执行 `cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j4` | **通过：55/55，约 4.38 秒**。不启用 GTK/WebKitGTK 桌面依赖；覆盖当前 lib 单元和集成 tests | 不代表 Android/桌面/iOS 产品流程完成；同快照 browser_harness E2E 证据见下行 |
| 当前 Rust 格式检查 | `cargo fmt --check`（root 对当前快照复核） | **通过** | 格式通过不代表运行时功能验收 |
| 当前前端构建 | `/workspace/legado_rs`: `npm run build`（package script 执行 `vue-tsc --noEmit && vite build`；root 报告当前快照通过） | **通过** | 前端类型检查/静态构建不代表 browser 用户流程完整可用 |
| DISC-01 收藏与真实 KMP 分类/分页 fixture | Linux headless；`source /workspace/.setup/activate.sh && source /workspace/.setup/native-env.sh && CARGO_TARGET_DIR=/tmp/legado-inventory-target cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib discovery::tests:: -- --nocapture --test-threads=1` | **通过：5/5**。实际 DesktopSourceExecutor/JNI/KMP 调用 `exploreKinds` 并读取第 1、2、3 页；验证第 1/2 页各返回预期卡片、第 3 页为空且 `hasNextPage=false`，确认请求页码正确、URL 只在私有映射、公共分类/结果 JSON 不含规则或分类 URL；另验证收藏添加/取消、标题更新后 ID 稳定和服务重启后持久 | 验证仅覆盖 Rust helper/服务与真实 KMP 解析链，不含收藏 Tauri command、Vue 发现 UI、各平台用户流程 |
| IMPORT-01 本地书导入后端 | Linux headless；工作目录 `/workspace/legado_rs/src-tauri`；`source /workspace/.setup/activate.sh && unset GTK_PATH GTK_EXE_PREFIX GTK_DATA_PREFIX PKG_CONFIG_PATH PKG_CONFIG_LIBDIR && CARGO_TARGET_DIR=/tmp/legado-rs-core-isolation-target cargo test --locked --no-default-features --lib local_books::tests -- --nocapture` | **通过：8/8**。覆盖 TXT/EPUB、CBZ 页自然排序与真实 loopback 图片读取、坏图片/zip-slip 不产生半成品、PDF metadata/页 HTML、实际 `application/pdf` 单 Range 206、密码与错误口令脱敏、页数上限及坏 PDF 拒绝 | 只验证 Rust 解析和资源后端；PDF/CBZ Tauri 文件选择/导入 UI、系统授权及目标平台操作仍未接入/验收 |
| 资源 JSON/HTML/媒体/PDF 资源服务定向测试 | Linux headless；工作目录 `/workspace/legado_rs/src-tauri`；`source /workspace/.setup/activate.sh && CARGO_TARGET_DIR=/tmp/legado-resource-check cargo test --locked --no-default-features --lib resources::` | **通过：14/14**（resource_core 报告）。包括发现收藏 JSON HTTP、PDF marker/asset MIME 与 Range 206、private media 持久化重启重载、稳定资源路径、安全、HTML/CSP 和媒体代理。后续 models roundtrip/theme tests 及其余 lib tests 纳入上面的当前 55/55 全套运行 | 资源服务测试不证明 Android/桌面/iOS WebView 用户链已可用 |
| Tauri desktop 类型检查 | desktop feature check；构建期间设置 `TAURI_CONFIG='{"bundle":{"resources":[]}}'` | **通过**（application_core 的集成验证）；跳过构建脚本在该环境打包 runtime resources 时遇到的 Permission denied 路径 | 类型检查不是桌面应用 bundle/启动验证；仍需真实打包与 WebView 资源读取 |
| SEARCH-01 早期浏览器运行（历史） | `/workspace/legado_rs`: `/workspace/.setup/browser-e2e-run-2026-10-03.log` | 早期真实浏览器运行完成搜索并显示结果；点击“加入书架”当时失败，详情覆盖层遮挡了目标按钮。该失败已由后续当前快照运行覆盖 | 保留作为修复前诊断记录，不代表最终快照结果 |
| SEARCH-01 当前快照 browser_harness E2E | 2026-10-03 02:18:04–02:18:15 UTC（北京时间 10:18）；freshly built `cargo --locked --no-default-features --example browser_harness`；结果目录 `/tmp/legado-browser-e2e-results/2026-10-03T02-18-04-040Z` | **通过：27 次真实 IPC**。KMP 搜索→加入书架；正文 36 段分页完整覆盖；字号 19→20 可保存且重开后仍为 20、章节 HTML hash 不变；切换已缓存章节没有 `content`/`prepare` IPC；进度重开恢复；人为制造 404 后修复并对同 URL 得到 200；私有书源 URL 返回 404。360px viewport 下目录 15px 行高 52、阅读标题 19px、状态栏 14px；章节按钮高 44–46px，header 位于 0–360px，页面无横向溢出 | 这是 desktop browser_harness 的核心阅读链证据，不验收 Android/iOS/桌面安装包产品流程，也不覆盖多源搜索、筛选/历史、取消/部分失败等高级搜索域 |
| Desktop host build | `source /workspace/.setup/activate.sh && source /workspace/.setup/native-env.sh && cargo check --locked --manifest-path src-tauri/Cargo.toml --example browser_harness -j4` | **通过**；browser_harness example 正常复制打包 resource/JRE 后构建成功 | 这是 harness/desktop 编译证据，不是安装包启动或完整 app 流程验收 |
| Android engine AAR | 设置 `ANDROID_HOME=/workspace/.setup/android-sdk`、`ANDROID_USER_HOME=/workspace/.setup/android-user`，`unset ANDROID_SDK_HOME` 后执行 `cd kotlin && ./gradlew --no-daemon --console=plain stageTauriAndroidAar` | **通过**：arm64、armv7、x86、x86_64 四 ABI 均构建并 stage AAR | APK 构建未成功；首次失败在 Gradle plugin resolution，`org.gradle.kotlin.kotlin-dsl:6.6.4` 未能从唯一配置 central mirror 获取。尚未安装/运行 Android APK |
| Android app package | `npm run tauri -- android build --debug --apk --ci` | **失败/阻塞**：Gradle dependency lookup 对 `org.gradle.kotlin.kotlin-dsl:6.6.4` 在已配置 central mirror 返回 404/not found；Gradle Portal POM 直连可返回 HTTP 200 | 排查允许的 Gradle repository/mirror 配置后重试；不据 AAR 构建通过宣称 Android app 可用 |

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
