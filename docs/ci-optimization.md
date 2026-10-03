# CI 与安装包优化记录

用户将 CI 速度、安装包大小与移除不必要编译设为最高优先级。优化保留 Android 四 ABI、桌面三平台及 iOS，不以删功能或跳过必要验证换取速度。PDF/漫画功能扩展暂缓。

## 优化前基线

GitHub Actions run `37109801382`，commit `a68c603`，2026-10-03 UTC。以下为该次实测，不代表冷缓存统一基准。

| Job | 总耗时 | KMP 构建 | 重复 cargo check | 最终打包 | 上传 |
|---|---:|---:|---:|---:|---:|
| macOS | 519 s | 125 s | 22 s | 284 s | 9 s |
| Linux | 748 s | 147 s | 7 s | 464 s | 33 s |
| Windows | 821 s | 196 s | 30 s | 490 s | 7 s |
| Android | 651 s | 248 s | — | 186 s | 21 s |

iOS 在 KMP 编译阶段失败，不能把未完成的构建耗时用作优化后成功构建的直接比较。

旧 Android 测试 APK（commit `2f57b33`）实际大小为 **1,249,314,614 bytes**。其四份 Rust 动态库未压缩：

| ABI | Rust .so bytes |
|---|---:|
| x86_64 | 322,028,504 |
| arm64-v8a | 315,645,832 |
| x86 | 299,268,064 |
| armeabi-v7a | 272,929,352 |

所有 native 库合计 1,236,233,312 bytes。最终 APK 大小须由新构建产物确认；对旧库 strip 的测量只能作为诊断，不能代替新包安装与功能验收。

## 验证要求

- 删除 Android AAR 前置 Rust 编译后，四种 QuickJS ABI 仍需存在；Rust 库由最终 Tauri APK 构建提供。
- 删除 iOS bootstrap 后，macOS CI 必须构建 device/simulator framework 并完成最终 app 链接；Linux dry-run 仅证明任务图。
- 缩减调试信息应保留 JNI/C ABI 动态导出，不删除平台支持。
- 上传范围只保留最终安装包，不重复上传 AppDir、解包 deb/rpm 等中间目录。
- 当前工作树无界面 Rust library 测试：101 passed、0 failed、0 ignored，5.90 s；该结果包含尚未提交的业务切片，不等于 CI 修改的跨平台验收。

## 前端取舍

当前 dist 为 2,098,017 bytes（逐文件 gzip 合计 633,958 bytes）。入口 JS 为 176,538 bytes；PDF 懒加载 chunk 与独立 worker 约占 dist 87.6%，只有打开 PDF 时才加载，不进入普通阅读入口。为减小安装包而删除它会损失已加入的功能，暂不采用。

桌面三个 job 各自重复执行前端构建，保留 Tauri beforeBuildCommand、删除前一次构建即可。独立共享 frontend artifact 会增加所有平台对前置 job 与下载的依赖；本机一次完整前端构建约 1.3 s，当前收益不足以抵消额外复杂度，暂不引入该流程。

## Android 库裁剪诊断

使用 Android NDK llvm-strip 测量旧 APK 内四 ABI Rust 库：总计 1,209,871,752 bytes；`--strip-debug` 后为 278,787,744 bytes（减少 77.0%）；`--strip-unneeded` 后为 161,581,944 bytes（减少 86.6%）。两种方式下各 ABI 均保留完全相同的 29 个动态导出，其中 26 个为 `Java_` JNI 符号。此测量未构造、签名或运行新 APK。

CI 测试包将关闭 Rust dev 调试信息并让 Android packaging 裁剪 native 符号；本地开发默认仍可保留调试符号。四 ABI 与 JNI 入口继续保留。

## 构建图变更的本机证据

Android KMP 强制重新编译通过（1m24s）；最终 AAR stage 通过（19 s，25 tasks：11 executed / 14 up-to-date），上游和 staged AAR 均只有四 ABI QuickJS、没有 Rust 库。桌面 JVM runtime 首次包含 Kotlin 编译的构建通过（1m9s），复跑 8 s、5/6 tasks up-to-date。iOS device/simulator dry-run 已无 Cargo bootstrap；实际 framework 与最终链接仍待 macOS CI。以上缓存条件不同，不能把本机耗时直接作为 CI 加速比例。

## CI 调度与产物范围

同一分支或 PR 的新运行取消旧运行；应用构建输入未变化时只保留变更检测，并明确记录平台 job 跳过。手动触发始终构建全平台；变更检测失败时退回全平台构建。

桌面保留 npm ci 和 Tauri 前端 hook，移除提前执行的同一前端 build 与 dev profile cargo check；最终 release 编译仍执行。上传仅含最终 AppImage/deb/rpm、dmg、msi/nsis exe。原 run `37109801382` 的 Linux artifact ZIP 为 792,199,818 bytes，其中含重复的中间目录；新上传大小待 CI。workflow 已通过 actionlint 1.7.12 与 YAML parse；Android 配置脚本已验证 CI/local 模式和重复执行，包内容检查已用合成 APK 验证。

## JVM 实际裁剪结果

最终 staged JRE 为 **74,061,673 bytes**（约71 MiB），完整 runtime 为 **95,541,686 bytes**（约91 MiB）；相比原 JRE 约99 MiB减少约28 MiB。模块集来自完整 classpath 的 jdeps，并保留动态发现所需字符集、EC provider、locale、JNDI DNS 和 Unsafe 等模块。

最终 staged runtime 上 `source_engine::tests` **2/2 通过**，实际执行 Rust→JNI→KMP 搜索、详情、目录、正文分页、HTTP/Cookie/重定向、QuickJS ajax 和 image.* 像素处理。运行时探针另验证 GB18030、EC keypair、BufferedImage、zh-CN locale、JNDI DNS、Unsafe、java.sql。Gradle 输入增加模块清单、JDK release/jlink/jmods 与 QuickJS native 文件，避免恢复过时 runtime。跨平台 jlink 与最终安装包仍由新 CI 验证。

## 新 CI 首次实测（run 37112338802）

commit `a9168dd` 的 Linux job 已通过，总耗时 **662 s**（基线748 s）。KMP runtime 192 s、最终 Tauri 打包336 s、上传9 s；缓存与 Gradle inputs 均有变化，该单次差异不能当作稳定加速比例。Linux artifact ZIP 为 **315,799,031 bytes**，较基线792,199,818 bytes减少 **60.14%**；此数值是上传归档大小，包含 JVM 裁剪与上传范围收窄的共同效果，不等于单个安装包大小减少60%。其余平台尚在执行。

新 Linux 安装包已下载并核对实际文件大小：AppImage **160,926,200 B**、deb **77,956,192 B**、rpm **77,936,568 B**。与本机保留的 commit `fe654d6` 包比较（并非同一基线run）：AppImage减少11.89%、deb减少21.96%、rpm减少21.99%。中间存在业务代码变更，因此该对比用于报告实际产物差异，不孤立归因到某一个选项。优化包原生启动/阅读检查进行中。

新 Android artifact ZIP 为 **67,742,465 B**（旧327,229,200 B，减少79.30%）。下载后的通用 debug APK 为 **180,399,822 B**（旧 `2f57b33` APK 1,249,314,614 B，减少85.56%），四种 ABI 均包含 Rust app 和 QuickJS `.so`。新 Rust 库大小分别为 arm64-v8a 41,132,496 B、armeabi-v7a 23,544,548 B、x86 49,296,364 B、x86_64 49,058,808 B；实际导出与签名检查进行中。

新 APK 验证通过 V2 签名，四 ABI ELF 架构均与目录一致。旧 Rust JNI 导出全部保留，新增 `nativeImageRequest`；QuickJS 导出集与旧包一致。两次 CI debug 证书不同：已有旧 debug APK 的设备可能需要卸载旧包再安装，不能把签名不匹配误判为新包损坏；持久数据应先备份。新 iOS device/simulator framework 与 XCFramework 已通过（framework阶段963 s），现进行 unsigned IPA 最终链接。

### 当前结果与速度限制

桌面三平台与 Android job 均成功；iOS framework/XCFramework 成功，最终 IPA 链接失败，缺少 QuickJS `JS_*` 符号。现有最终 Rust link 已带 mbedTLS/sqlite，需同样加入生成的 libquickjs.a；不恢复冗余 Rust bootstrap。

macOS job 1,068 s、Windows job 1,145 s，均比旧基线长；本轮不能宣称整体 CI 已提速。完整日志显示新 Gradle key 未命中、`work` 分支 cache 为 read-only，缓存摘要0 restored/0 saved；这会导致后续相同分支也不能保存新 Gradle 缓存。Cargo manifest 修改也使 Rust cache 部分恢复、重新编译。后续修复 trusted push 的 Gradle 缓存写入，并让 Android debug profile 对 Rust cache action 可见。Basic Caching 升级提示本身只提供服务选择，不要求切换商业增强缓存。

新 macOS artifact ZIP为71,929,169 B（旧189,752,074 B，减少62.09%）；Windows为139,444,787 B（旧183,969,164 B，减少24.20%）。Linux新AppImage实际原生流程通过：GTK picker导入TXT、第二章翻到第2/2页、退出重启后恢复相同章节和页；无FUSE环境使用内置extract启动。


## iOS 最终 Xcode 链接与 framework 精确缓存

run `37113999486`（commit `38913f5`）的四个非 iOS job 均通过。iOS device/simulator framework 编译通过（751 s），XCFramework 打包通过，Rust QuickJS 链接也通过；最终 Xcode app 链接仍失败：找不到 `LegadoSourceEngine` framework，缺少 `_OBJC_CLASS_$_LSEIosSourceEngine`。修复为 Tauri 官方 iOS 模板加入 SDK 条件的 framework 搜索路径，并声明最终 app framework 依赖，仍须下一次 macOS CI 确认最终 archive/IPA。

新增 exact-only iOS framework/native archive 缓存：键包含源码、原生工具链输入、Xcode/SDK 版本及目标；不接受部分恢复。命中后检查 device/simulator slice 和 arm64 archives，未命中构建后在 IPA 步骤前保存，避免最终 app 失败丢掉成功的 KMP 编译结果。生成 build 输出不参与键，保存使用 restore 时的固定 key。

该次 macOS job 为357 s，比上一轮1,068 s缩短；Rust exact cache 命中，Gradle首次保存新缓存。缓存条件不同，该单次结果不能作为统一冷构建加速比例。Basic Caching 信息提示无需升级付费缓存。


最终 Xcode 配置复核进一步改为显式本地 XCFramework dependency（`framework: ../../plugins/source-engine/ios/Frameworks/LegadoSourceEngine.xcframework`, `embed: false`），避免 `sdk:` 把自定义 framework 当作 SDK 内置路径。模板配置为 `src-tauri/ios-project.yml`，匹配 workflow 在仓库根运行 `tauri ios init` 的实际模板读取路径；SDK 条件搜索路径保留。两项仍待 run `37116333002` 的 macOS 初始化和最终 IPA 验证。


run `37116777187`（`da28565`）四个非iOS平台job均通过；iOS框架构建、校验和Rust链接成功，最终Xcode app仍缺 `_sqlite3_*`。模板加入 SDK `libsqlite3.tbd`，让最终app满足现有KMP静态framework的链接依赖。独立引擎 `IosSourceEngineRuntime` 使用HeadlessAppDb，书架/进度等DAO不可用，cache/cookie通过Rust HostStorage；框架含旧SQLite driver符号不表示新app使用SQL业务存储，新app仍持久化JSON/HTML。新补链还须下轮macOS实际IPA验证。
