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
| RSS-01 RSS 状态 helper | Linux headless；`/workspace/legado_rs/src-tauri`；`source /workspace/.setup/activate.sh && export CARGO_INCREMENTAL=0 && cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib rss:: -j2 -- --nocapture` | **通过：4/4**。覆盖稳定文章/分类 ID、read/favorite/filter 状态与 schema 拒绝、普通 Atom 处理后的 HTML 资源；断言正文不进入公开卡片或私有 search JSON，状态 helper 在重启后保持身份语义 | 仅 Rust helper/服务测试；尚无 RSS Tauri commands、订阅 UI、端到端刷新/退订或平台验收。标准 feed 先全量过滤再分页；旧 KMP RSS 仅过滤引擎返回页 |
| APP-01 首页配置 helper | Linux headless；`/workspace/legado_rs/src-tauri`；`source /workspace/.setup/activate.sh && export CARGO_INCREMENTAL=0 && cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib discovery::home_config:: -j2 -- --nocapture` | **通过：2/2**。覆盖默认配置、opaque source/category ID 对应私有映射校验、metadata 投影、禁止 URL/rules 字段以及 JSON 重启持久化 | 仅 Rust helper/服务测试；Tauri 命令、Vue 首页 tab/section 编辑和 Android/桌面/iOS 用户流程尚未接入或验收 |
| LIB-01 | `/workspace/legado_rs/src-tauri`: `cargo test --no-default-features --lib shelf_groups_and_sort_modify_the_shared_shelf_json -- --nocapture` | **Rust 测试通过（1/1）**。书架排序、每书分组、顶层空分组 registry、重命名/删除和 ResourceStore reopen 后保留空分组 | Vue 空组管理、并发的进度更新/预取整合和跨平台 UI 流程仍待验收 |
| 全套 Rust 测试（修复前记录） | `/workspace/.setup/resumed-core-tests.log` | **历史运行：21/22**；唯一失败为 EPUB 相对 media path assertion，随后修复。仅保留作诊断轨迹 | 最新结果见下一行；还需实际 Vue 阅读生命周期、统计展示、书签导航验证 |
| 全部已实现 Rust 单元/集成覆盖（较早快照；headless） | 2026-10-03 `/workspace/legado_rs/src-tauri`: `source /workspace/.setup/activate.sh && cargo test --no-default-features --lib` | **当次通过：34/34，0 失败**。覆盖 backup、初期 discovery projection/ID、TXT/EPUB、书签与阅读历史日统计、分组/排序、资源安全/HTML/media proxy、标准 Atom RSS fixture、Rust→KMP search/detail/catalog/content fixture、稳定资源服务。此次 `--no-default-features` 不依赖 GTK/WebKitGTK | 此为添加 discovery KMP fixture、CBZ/PDF 和资源扩展测试前的运行记录，不代表当前全套 suite 已重跑。Rust headless 验证也不代表 Vue E2E 或桌面/Android/iOS app 已验收。此前 21/22 的运行记录见 `/workspace/.setup/resumed-core-tests.log` |
| Rust locked headless 重跑（较早快照） | `cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j4` | **当次通过：34/34**；application_core 独立复核 lockfile 下结果 | 当前 suite 后续增加了 discovery/local-book/resource tests；尚未以一次全量运行重新记录总数 |
| Rust headless 全套（历史快照） | 2026-10-03；root 对较早共享快照执行 `CARGO_INCREMENTAL=0 cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j2`，未加载 native GTK 环境 | **该快照通过：81/81，0 failed/ignored/filtered，约 3.34 秒，exit 0**。之后又加入 home/RSS 命令、redirect 单测等改动，因此不是当前覆盖数 | 当前快照总数待 root 对已去重测试执行全套重跑后记录；任何一次 headless suite 都不代表 Android/桌面/iOS 产品流程完成 |
| Rust headless 全套（本次后端提交快照） | 2026-10-03；root 执行 `CARGO_INCREMENTAL=0 cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j2`；日志 `/tmp/legado-headless-current.log` | **通过：87/87，0 failed/ignored/filtered，3.77 秒，exit 0**。已移除重复 home module；包含新 home/RSS/TXT service API、真实 KMP POST 重定向与正文分页、HTTP 多跳/零重定向限制、PDF challenge 生命周期等覆盖 | 没有启用 GTK/WebKitGTK。新增 API 的 Vue 用户流程和 iOS NativeJS 桥运行仍待验收；此结果不代表整个 app 完成 |
| 当前 Rust 格式检查 | `cargo fmt --check`（root 对当前快照复核） | **通过** | 格式通过不代表运行时功能验收 |
| 当前前端构建 | `/workspace/legado_rs`: `npm run build`（package script 执行 `vue-tsc --noEmit && vite build`；root 报告当前快照通过） | **通过** | 前端类型检查/静态构建不代表 browser 用户流程完整可用 |
| DISC-01 收藏与真实 KMP 分类/分页 fixture | Linux headless；`source /workspace/.setup/activate.sh && source /workspace/.setup/native-env.sh && CARGO_TARGET_DIR=/tmp/legado-inventory-target cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib discovery::tests:: -- --nocapture --test-threads=1` | **通过：5/5**。实际 DesktopSourceExecutor/JNI/KMP 调用 `exploreKinds` 并读取第 1、2、3 页；验证第 1/2 页各返回预期卡片、第 3 页为空且 `hasNextPage=false`，确认请求页码正确、URL 只在私有映射、公共分类/结果 JSON 不含规则或分类 URL；另验证收藏添加/取消、标题更新后 ID 稳定和服务重启后持久 | 验证仅覆盖 Rust helper/服务与真实 KMP 解析链，不含收藏 Tauri command、Vue 发现 UI、各平台用户流程 |
| IMPORT-01 本地书导入后端 | Linux headless；工作目录 `/workspace/legado_rs/src-tauri`；`source /workspace/.setup/activate.sh && unset GTK_PATH GTK_EXE_PREFIX GTK_DATA_PREFIX PKG_CONFIG_PATH PKG_CONFIG_LIBDIR && CARGO_TARGET_DIR=/tmp/legado-rs-core-isolation-target cargo test --locked --no-default-features --lib local_books::tests -- --nocapture` | **通过：8/8**。覆盖 TXT/EPUB、CBZ 页自然排序与真实 loopback 图片读取、坏图片/zip-slip 不产生半成品、PDF metadata/页 HTML、实际 `application/pdf` 单 Range 206、密码与错误口令脱敏、页数上限及坏 PDF 拒绝 | 只验证 Rust 解析和资源后端；PDF/CBZ Tauri 文件选择/导入 UI、系统授权及目标平台操作仍未接入/验收 |
| 资源 JSON/HTML/媒体/PDF 资源服务定向测试 | Linux headless；工作目录 `/workspace/legado_rs/src-tauri`；`source /workspace/.setup/activate.sh && CARGO_TARGET_DIR=/tmp/legado-resource-check cargo test --locked --no-default-features --lib resources::` | **通过：14/14**（resource_core 报告）。包括发现收藏 JSON HTTP、PDF marker/asset MIME 与 Range 206、private media 持久化重启重载、稳定资源路径、安全、HTML/CSP 和媒体代理。后续 models roundtrip/theme tests 及其余 lib tests 纳入上面的当前 81/81 全套运行 | 资源服务测试不证明 Android/桌面/iOS WebView 用户链已可用 |
| SOURCE-04 Rust ImageOps 与跨桥接 fixture | 2026-10-03 当前 Rust headless 验证；包含在上述锁定 no-default 全套测试中 | **ImageOps 7/7、C ABI 1/1、真实 JNI/KMP 书源脚本 image fixture 1/1 通过**。覆盖图像像素操作协议、C ABI 返回/释放和脚本经 JVM/JNI 调用 Rust 图像操作 | Rust/JVM JNI 路径通过不代表 iOS NativeJS ImageOps 桥已实现或验证；书源兼容矩阵和各平台真实规则流程仍待验收 |
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
| 无界面依赖边界 | Linux Cargo feature graph；root 对当前依赖树的检查 | `cargo tree --locked --no-default-features --edges normal` 和 `cargo tree --locked --no-default-features --edges build` 均未发现 GTK/WebKitGTK 匹配项；headless Rust suite 可不加载 GTK native 环境运行 | 纯逻辑测试不编译桌面 WebView 依赖；真实默认桌面和移动运行时仍须在各自平台构建验证，这项检查不代替完整 app 构建。 |
| Android 原生包启动（旧快照，API 36 系统阻塞） | Linux 软件模拟器；临时 AVD `LegadoApi36Aosp`，Android 16/API 36 AOSP x86_64；GitHub run `37095535654` / commit `2f57b33` 的 universal debug APK（约 1.25 GB） | 2 GiB guest 下安装期间 PackageManager 返回 `Broken pipe`，APK 未安装。将仅 `/tmp` 的 AVD 配置调至 4 GiB 后，guest 曾达到 `BOOT_COMPLETED`，但约 3 秒后 `system_server` 抛出 `IllegalStateException: Lost network stack`，PackageManager/ActivityManager 服务消失；APK 仍未安装，未启动任何 app 进程。guest dmesg/logcat 没有 OOM/low-memory kill 证据；4 GiB 检查时 guest `MemAvailable` 约 3.45 GiB。 | 这是 Android emulator/system 服务阻塞，不是 app 崩溃，也不能记为 Android app 通过或失败。此旧快照不验证后续 PDF/CBZ UI。API 35 替代镜像尝试见下一行。 |
| Android 原生包启动（旧快照，API 35 首启系统阻塞） | Linux 软件模拟器；仅 `/tmp` 的官方 API 35 Google APIs x86_64 system image（SDK archive 1,738,815,903 B，通过 sdkmanager 的 TLS/checksum 验证安装）；`LegadoApi35Google` AVD，4 GiB guest RAM、4 GiB data 分区、无 KVM 软件 TCG；GitHub run `37095535654` / commit `2f57b33` 的 universal debug APK（1,249,314,614 B） | Android 15/API 35 guest 在约 5 分钟后启动 PackageManager/ActivityManager 和 NetworkStack；首次 ART dexopt 继续进行。`StartServices` 共耗时约 501 秒后，Watchdog 记录主线程阻塞 61 秒并 SIGKILL `system_server`；随后 PackageManager/ActivityManager 消失，其他系统进程报告 `DeadSystemException`。NetworkStack 没有先行崩溃。APK 未安装，app 未启动。Guest dmesg 因权限拒绝不可读；logcat 没有 OOM/low-memory kill 证据；lmkd 断开日志发生在 system_server 被杀后。 | 这是无 KVM TCG 下的 Android 首启/framework 阻塞，不是 app 崩溃，也不能记为 Android app 通过或失败。尝试后已停止该 AVD，不再重试；日志 `/tmp/legado-api35-avd/emulator.log`。需要在支持 KVM 的 runner、实体 Android 设备或更可靠的平台环境执行原生 APK/picker 流程。旧 APK 不验证当前源码或后续 PDF/CBZ UI。 |
| Desktop 原生文件选择、TXT 导入、阅读样式和进度重开（旧快照） | Linux Xvfb `:99` + 真实 GTK/WebKit AppDir；commit `fe654d6` 旧 Linux artifact；数据隔离到 `/tmp/legado-native-fe654d6` | 使用可复用启动脚本 `/workspace/.setup/native-session.sh` 启动 AppDir，通过 GTK portal chooser 选择 `/tmp/legado-native-fe654d6/localflowsample.txt`。实际导入 8.2 KB UTF-8 中文 TXT，书架出现本书与 4 章目录（含自动序章）；打开“第二章 阅读进度保存”，翻到第 2/3 页，将字号改为 20 px 并保存，退出后重启 app；首页继续阅读再次打开同一章第 2/3 页，阅读设置面板恢复为 20 px。重启后再次打开原生 GTK 文件选择器也成功。磁盘 `progress/{bookId}.json` 显示 `chapter-00003` / index 2 / offset 1；`settings.json` 中 `fontSizePx` 为 20。 | **通过：仅验证 commit `fe654d6` 的旧 Linux artifact，不代表当前未提交源码或 PDF/CBZ UI。** 启动时将 sysroot 中的 portal、GTK backend、PermissionStore 和 Documents service 以隔离 D-Bus service 文件指向 `/workspace/.setup/sysroot` 的真实路径；GTK file chooser 和本地路径选择实际成功。容器没有 `/dev/fuse`，Document portal 日志有挂载警告，但不妨碍此次 FileChooser 返回文件。截图和日志在 `/tmp/legado-native-fe654d6/`。 |

以上均为核心实现级验证，不代表完整 app 或三平台用户流程已验收。

## 2026-10-03 HTTP redirect 与真实书源验证

以下测试使用临时本地数据目录和已授权书源；不保存书源 JSON、规则、章节正文或完整远端 URL 到仓库。真实书源证据仅记录返回状态、数量和资源完整性。

| 流程 | 平台 / 环境 | 命令或操作 | 实际结果 | 限制及后续动作 |
|---|---|---|---|---|
| Rust→KMP POST 搜索重定向回归 | Linux headless + freshly staged JVM KMP runtime | `/workspace/legado_rs`: `cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j2 source_engine::tests::post_search_redirect_uses_real_final_url_and_reads_paginated_book_content -- --nocapture` | **通过：1/1**。本地 fixture 真实返回 303；Rust DTO 保留真实跳转状态/来源/目标；KMP 将最终详情地址作为书籍 URL，且没有搜索模板；POST body 断言关键字已展开。继续执行详情、`nextTocUrl` 两页目录（3 章）和选中章节的两页 `nextContentUrl` 正文 | fixture 验证 Desktop/JVM KMP 路径；Android/iOS provider 的包级/设备端运行仍需平台构建和运行验证 |
| Rust HTTP 重定向 trace 与限制 | Linux headless；本地 HTTP fixture | `/workspace/legado_rs`: `cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib -j2 source_http::tests::redirect_trace_is_ordered_request_local_and_keeps_limit_semantics -- --nocapture` | **通过：1/1**。按顺序验证实际 301→307 两跳；`maxRedirects=0` 保持原 reqwest 行为（返回跟随重定向错误）；后续普通请求的跳转 trace 为空，不串用前一请求数据 | reqwest 跨主机敏感 header 清理和现有限跳策略保持由 reqwest 实施；此 fixture 未单独模拟跨主机敏感 header |
| 用户提供书源：搜索和加入书架/目录 | Linux `browser_harness` 的 `ApplicationService` HTTP API（未启动 Vue/WebView）；新进程、新临时数据目录；runtime JAR SHA-256 `839be9bd3260dedba3d9d26a7e4e0d6a1cfad6e221ec8dc042df72a7d020220d` | 通过 harness 导入 `/tmp/legado-user-source-sudugu.json`，调用 `search_books`（准确关键词 `没钱修什么仙`），再调用 `add_book` 并 GET 处理后的书籍资源 | 搜索 HTTP 200、1 个结果、0 个来源错误；处理结果中的书籍 URL 为干净详情 URL（没有 `{{key}}` 或 POST 搜索负载）；加入书架和书籍/目录 JSON GET 均 HTTP 200，目录列出 1062 章 | 为了确认目录是否返回完整，`add_book` 解析了该源返回的整个目录；后续只下载/缓存了第 0 章，没有批量取正文。当前检查没有证明 `ContentRule.webJs` 已执行；普通 WebBook 的 `BookContent` 路径不读取该字段，下一行只确认 `nextContentUrl` 分页。Vue/WebView 用户流程未运行 |
| 用户提供书源：单章多页正文 HTML | 同一全新 JVM/harness 运行；仅请求 `prepare_chapters(fromIndex=0,count=1)` | GET 返回的处理后章节 `src` | **通过：**章节资源 HTTP 200，`text/html; charset=utf-8`，15,234 字节，含非空 HTML body；经 harness 日志的安全摘要确认第 2–6 页完成、共 6 页，表明 `nextContentUrl` 链成功读取 | 仅缓存一个章节；没有把章节正文写入验证记录。该源 `ContentRule.webJs` 字段存在，但旧 legado 与当前普通 WebBook 内容路径均未引用它；不声称本流程执行了此字段中的脚本。iOS/Android 对应 provider 仍待目标平台运行验证 |

以上 live/source 记录确认了该授权书源搜索关键字展开、重定向详情 URL、目录产物和首章多页 HTML 资源；不覆盖全平台、其余 1061 章的缓存或 `ContentRule.webJs` 行为，也不表示整个 app 已完成。

## 2026-10-03 KMP Gradle 构建图优化

| 检查 | 平台 / 环境 | 命令或操作 | 实际结果 | 限制及后续动作 |
|---|---|---|---|---|
| Android KMP 编译 | Linux；KMP 模块；`ANDROID_HOME=/workspace/.setup/android-sdk`；Java 21，Gradle toolchain 可见 JDK 17/21 | `source /workspace/.setup/activate.sh && ANDROID_HOME=/workspace/.setup/android-sdk ANDROID_USER_HOME=/workspace/.setup/android-user JAVA_HOME=/workspace/.setup/jdk21 ./gradlew --no-daemon --console=plain -Dorg.gradle.java.installations.paths=/workspace/.setup/jdk17,/workspace/.setup/jdk21 --max-workers=2 :kmp-engine:compileAndroidMain --rerun-tasks` | **通过，exit 0**；Android KMP Kotlin 编译执行完成，BUILD SUCCESSFUL，1m24s | 不代表 APK 构建/安装/运行或 Android 设备流程验收 |
| Android Tauri KMP AAR | 同一 Linux 环境，Android NDK/CMake/Ninja 已配置 | `source /workspace/.setup/activate.sh && ANDROID_HOME=/workspace/.setup/android-sdk ANDROID_USER_HOME=/workspace/.setup/android-user JAVA_HOME=/workspace/.setup/jdk21 CMAKE_BUILD_PARALLEL_LEVEL=2 ./gradlew --no-daemon --console=plain -Dorg.gradle.java.installations.paths=/workspace/.setup/jdk17,/workspace/.setup/jdk21 --max-workers=2 stageTauriAndroidAar` | **通过，exit 0**；BUILD SUCCESSFUL，19s，25 actionable tasks（11 executed、14 up-to-date）。四 ABI QuickJS 均保留；bundle AAR 和 Tauri staged AAR ZIP 均有 `arm64-v8a`、`armeabi-v7a`、`x86`、`x86_64` 的 `liblegado_quickjs.so`，均无 `liblegado_lib.so`。仅清除 KMP `build/generated/androidJniLibs` 下四个旧 Rust staging 文件；最终 stage filter 仍保留。 | 验证 AAR/ABI 内容和构建图，不覆盖 Tauri APK 的 Rust 构建、安装或设备运行 |
| Desktop JVM runtime | Linux；KMP JVM 编译与 runtime staging | `source /workspace/.setup/activate.sh && JAVA_HOME=/workspace/.setup/jdk21 ./gradlew --no-daemon --console=plain -Dorg.gradle.java.installations.paths=/workspace/.setup/jdk17,/workspace/.setup/jdk21 --max-workers=2 prepareDesktopJvmRuntime` | **通过，exit 0**。初次最终目标执行包含 `compileKotlinJvm` / `jvmJar`，1m9s；随后复跑为 BUILD SUCCESSFUL、8s，5/6 tasks up-to-date | 只验证 JVM engine/runtime 准备，不等于桌面安装包或应用用户流程验收 |
| iOS KMP framework 构建图 | Linux dry-run；iOS Kotlin/Native target 因 Apple cinterop 在此主机禁用 | `source /workspace/.setup/activate.sh && JAVA_HOME=/workspace/.setup/jdk21 ./gradlew --no-daemon --console=plain -Dorg.gradle.java.installations.paths=/workspace/.setup/jdk17,/workspace/.setup/jdk21 --max-workers=2 --dry-run :kmp-engine:linkReleaseFrameworkIosArm64 :kmp-engine:linkReleaseFrameworkIosSimulatorArm64` | **dry-run 通过，exit 0**；device/simulator framework graph 不再调度 Cargo Rust bootstrap task，只包含共享的 `buildIosNativeQuickJs`，保留两个 target 的 cinterop/KSP/compile/link task graph | Linux dry-run 不会链接 framework。必须由 Mac CI 实际构建 device 与 simulator framework、打包 XCFramework、构建 unsigned IPA，并确认最终 app 的 Rust C ABI symbols 已解析且无 duplicate；当前 iOS 功能状态未验收 |

## 2026-10-03 优化后 Linux AppImage 原生导入与进度恢复

| 检查 | 平台 / 环境 | 命令或操作 | 实际结果 | 限制及后续动作 |
|---|---|---|---|---|
| Linux AppImage 启动、原生 TXT 导入、阅读进度重开 | x86_64；CI artifact commit `a9168dd`，AppImage 160,926,200 B，SHA-256 `6b54582fa31ceaa992cc00b0bdccbfc668354ee93021007e0caad5b9fe7a0f9b`；独立 Xvfb `:98` 与 `/tmp/legado-native-a9168dd` XDG 数据目录 | `./legado_0.1.0_amd64.AppImage --appimage-extract`；用 `/workspace/.setup/native-session.sh -- /tmp/legado-ci-optimized-linux/appimage/squashfs-root/AppRun` 启动提取的 `AppRun`。通过 GTK 原生文件选择器导入 `/tmp/legado-native-fe654d6/LocalFlowSample.txt`，打开“第二章 阅读进度保存”，翻到 2/2，返回书籍详情后重启同一 AppRun，并点击“继续阅读” | **通过**：新安装包实际显示主界面；GTK 文件选择器完成 TXT 选择；书籍目录解析出 4 章；重启后恢复“第二章 阅读进度保存”第 2/2 页。持久化 JSON 为 `chapterId=chapter-00003`、`chapterIndex=2`、`offset=1` | 因此环境没有 FUSE，先用 AppImage 内置 `--appimage-extract` 再启动其 `AppRun`；验证覆盖解包后的桌面应用、原生 GTK chooser、TXT 阅读与进度恢复，不覆盖 AppImage FUSE 挂载行为。portal 日志的 document-FUSE 警告未阻断本地文件选择 |

## 2026-10-03 阅读器缓存调度与目录刷新浏览器端到端

| 检查 | 平台 / 环境 | 命令或操作 | 实际结果 | 限制及后续动作 |
|---|---|---|---|---|
| 当前章单章缓存、后台预取失败恢复、目录重排进度保持 | Linux Chromium；当前未提交工作树；Vite production dist `/tmp/legado-browser-e2e-dist-38913f5`；新构建 no-default-features Rust `browser_harness`，真实 JVM/KMP source engine；本地 HTTP source fixture 与独立临时应用数据目录 | `source /workspace/.setup/activate.sh && npx vue-tsc --noEmit && npx vite build --outDir /tmp/legado-browser-e2e-dist-38913f5`；`source /workspace/.setup/activate.sh && cargo build --locked --no-default-features --example browser_harness`；`source /workspace/.setup/activate.sh && BROWSER_E2E_DIST=/tmp/legado-browser-e2e-dist-38913f5 BROWSER_HARNESS_BINARY=/workspace/legado_rs/src-tauri/target/debug/examples/browser_harness PLAYWRIGHT_OUTPUT_DIR=/tmp/legado-browser-e2e-results/2026-10-03-cache-catalog-e2e-38913f5 node scripts/e2e-browser.mjs` | **通过**：类型检查、生产前端构建和浏览器 E2E 均 exit 0；Chromium 完成 41 次 Rust IPC。首章只请求 `(fromIndex=0,count=1)` 并可立即阅读；下一章预取在上游请求挂起时，离开阅读器仍成功保存首章 `chapterId/index=0/offset=11`；首次和重新打开后的后台请求分别遇到受控 HTTP 503/KMP `ContentEmptyException`，当前章仍可读，下一章未发布缓存 URL；用户显式翻章后第三次真实 KMP 请求成功。流程随后重开并修复缺失缓存 URL（404→200），最后以真实 KMP 目录刷新得到 `[2,1]` 顺序，章节 ID 保持稳定，已读第二章进度由 `chapterIndex=1` 映射至 `0` 且 `offset=2` 不变；重开后恢复同章同页且未额外请求正文。所有 36 个长章节 fixture 段落通过翻页检查；显示字号变化没有改写 HTML cache | 浏览器与真正 Rust/KMP 服务、JSON/HTML resource server 和应用持久化交互；测试 IPC/events 通过同源 Node adapter，避免 Chromium 的 isolated local-network request 限制，章节资源仍从真实 loopback resource server 直接 GET。此项不是 GTK/WebKitGTK、Android 或 iOS 运行时验证。E2E 机器证据和截图位于 `/tmp/legado-browser-e2e-results/2026-10-03-cache-catalog-e2e-38913f5/`；PDF 的 `ERR_ABORTED` 仍按前文记录为失败，未被此小说流程覆盖 |
