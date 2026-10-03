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
