# Foliver 开发与打包

JDK、Kotlin、KSP、Gradle、AGP 与 Android SDK/NDK 的版本统一维护在 [toolchain.properties](../src-tauri/plugins/source-engine/toolchain.properties)，App、插件和 KMP 使用同一套配置。修改版本后运行 `node src-tauri/toolchain.mjs --sync`，同步 KMP 的 Gradle wrapper 与 daemon JDK；已有 Android 工程再运行 `node src-tauri/android/configure.mjs`。CI 自动执行。手动打包见 [build.yml](../.github/workflows/build.yml)，版本发布见 [release.yml](../.github/workflows/release.yml)。

构建使用 JDK 21；桌面 JVM 字节码目标为 21，Android 为 11。Gradle 自动选择 JDK 21，不需要修改全局 `JAVA_HOME`。Android CMake 和 NDK 必须安装配置中指定的版本，不再自动选择已安装的最高版本。

## 各平台的运行时

| 平台 | 书源引擎 | 分发方式 |
|---|---|---|
| Windows、macOS、Linux | Kotlin/JVM + QuickJS JNI | 附带精简的 Java 21 运行时。 |
| Android | 系统 ART + QuickJS NDK 动态库 | 按 ABI 分发 Release APK。 |
| iOS | Kotlin/Native + QuickJS/mbedTLS 静态库 | 真机 IPA。 |

## 桌面

需要 Node.js 24、Rust stable、JDK 21、CMake 和 C/C++ 工具链。Linux 系统依赖见 CI。

Windows 使用 `x86_64-pc-windows-gnu` 时，需要在 `PATH` 中提供 MinGW 的 `gcc`、`g++`；项目会为 CMake 指定这套编译器，避免误选面向 MSVC 的 Clang。已有错误编译器缓存时，先运行 `cargo clean --manifest-path src-tauri/Cargo.toml -p opusic-sys` 再重新构建。

```sh
npm ci
bash src-tauri/plugins/source-engine/engine/gradlew -p src-tauri/plugins/source-engine/engine --no-daemon --console=plain prepareDesktopJvmRuntime
npm run tauri -- dev
```

Windows 的 Gradle 命令使用 `.\src-tauri\plugins\source-engine\engine\gradlew.bat -p src-tauri/plugins/source-engine/engine --no-daemon --console=plain prepareDesktopJvmRuntime`。修改 Kotlin 后需重新准备 runtime。桌面打包使用 `npm run tauri -- build`；CI 分别生成 Windows NSIS、macOS DMG、Linux AppImage/DEB。

## Android

需要 Android SDK、NDK 与 Rust 的 `aarch64-linux-android` target，最低 Android 8（API 26）。

```sh
bash src-tauri/plugins/source-engine/engine/gradlew -p src-tauri/plugins/source-engine/engine --no-daemon --console=plain stageTauriAndroidAar
npm ci
npm run tauri -- android init --ci
npm run icons
node src-tauri/android/configure.mjs
npm run tauri -- android build --apk --ci --split-per-abi
```

调试构建添加 `--debug`。生成的 Android 项目由 `src-tauri/android/configure.mjs` 配置。

## iOS

需要 macOS 与 Xcode。

```sh
export IPHONEOS_DEPLOYMENT_TARGET=15.0
bash src-tauri/plugins/source-engine/engine/gradlew -p src-tauri/plugins/source-engine/engine --no-daemon --console=plain -Plegado.iosNativeTargets=ios_arm64 :kmp-engine:linkReleaseFrameworkIosArm64
xcodebuild -create-xcframework -framework src-tauri/plugins/source-engine/engine/kmp-engine/build/bin/iosArm64/releaseFramework/LegadoSourceEngine.framework -output src-tauri/plugins/source-engine/ios/Frameworks/LegadoSourceEngine.xcframework
npm ci
npm run tauri -- ios init --ci
npm run icons
npm run tauri -- ios build --no-sign --ci
```

重复组装 XCFramework 前需移走旧输出。模拟器开发可不传 `legado.iosNativeTargets`，使用 `:kmp-engine:linkReleaseFrameworkIosSimulatorArm64`。未签名 IPA 需签名后安装。

## 版本发布

工作流职责：

- **CI**：PR 和 `main` 提交检查工作流语法、发版逻辑、前端构建、JVM 引擎及 Rust 编译，无需签名 Secret。
- **Package Foliver**：手动选择平台和目标级别，只保存 Actions 附件，不创建 Release。
- **Release Foliver**：校验版本与签名配置，执行工作流检查和发版测试，构建所选目标，汇总校验附件后创建标签并发布 Release。实际打包已经包含编译，不再重复完整 Linux CI。

### 手动发版

打开 Actions → Release Foliver → Run workflow，选择源码分支或已有标签，以及 `tiers`（默认 `1`）。工作流从 `package.json` 读取版本，与 npm 锁文件、Tauri、Cargo 及 Cargo 锁文件核对，固定本次源码 SHA。**不需要填写或提前创建版本标签**。例如所选源码版本为 `0.1.0`，所选目标全部构建成功后会自动创建 `v0.1.0` 标签并发布。

默认公开发布；勾选 `draft` 时仅保留已校验的草稿，随后可在 GitHub 手动发布。已公开的同名版本不重复构建或覆盖，已有标签不允许移动到其他源码。需要发新版本时先同步版本、提交，再运行工作流。

### 标签发版

推送 `vX.Y.Z` 标签自动构建一级目标并发版，补丁版本正常支持。`v1.0.0-beta.1`、`v1.0.0-rc.1` 等会标记为 GitHub Prerelease。需要二、三级时使用手动入口选择。标签必须与源码中的应用版本一致。标准稳定版本号与预发布版本号均不允许前导零；不使用构建元数据 `+build` 作为发版版本。

用 Python 3.11+ 同步版本，提交后推送标签：

```sh
python scripts/release.py version 0.1.1
python scripts/release.py check v0.1.1
git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: release v0.1.1"
git tag -a v0.1.1 -m "Foliver v0.1.1"
git push origin HEAD
git push origin v0.1.1
```

不传标签运行 `python scripts/release.py check` 可检查当前应用版本。预发布可使用 `python scripts/release.py version 1.0.0-rc.1`。CI 创建标签使用 `GITHUB_TOKEN`，不会递归触发另一轮标签发版。

### 构建、签名与发布

所有架构固定使用同一源码提交，构建 job 只读仓库。目标分三级，定义统一保存在 [build-targets.json](../scripts/build-targets.json)，构建矩阵、产物校验和更新清单共用该文件：

| 级别 | 目标 | 用途 |
|---|---|---|
| 一级 | Windows x64、macOS ARM64、Linux x64、Android ARM64、iOS ARM64 真机 | 常规发版，5 个原生构建任务 |
| 二级 | Android ARMv7、macOS Intel x64、Windows ARM64、Linux ARM64 | 老手机、部分阅读设备、存量 Intel Mac、在售 ARM 笔记本及补充桌面设备，4 个任务 |
| 三级 | Android x86_64、x86 | 模拟器及特殊设备场景，2 个任务，不作为常规手机发版目标 |

级别相互独立：`tiers=2` 只构建二级，`tiers=3` 只构建三级，不附带一级；可选择 `1,2`、`1,3`、`2,3` 或 `1,2,3` 组合。Package 还可用 `platform` 取平台交集，例如 `tiers=2, platform=android` 仅构建 ARMv7；没有匹配目标时在启动原生 runner 前报错。未选级别不创建原生构建任务，也不参与完整性校验。相同版本需包含多个级别时一次选择对应组合；已公开版本仍不覆盖，补充测试包可通过 Package 独立构建。

Windows 使用 NSIS `.exe`，macOS 使用 `.dmg`，Linux 提供 `.AppImage` 和 `.deb`，Android 提供签名 `.apk`，iOS 提供未签名 `.ipa`。这套分级决定默认构建范围，保留全部目标的按需构建能力，并不将 ARM 笔记本归为淘汰设备。

前端与架构无关，同次打包只编译一次前端，生成共享 `frontend-dist`，各原生任务下载后使用 `tauri.ci.conf.json` 跳过重复的前端编译。原生任务仍安装自身所需的 Tauri CLI，使用 npm 缓存；Rust 依赖在编译失败时也保存缓存，减少重跑成本。各原生任务没有相互依赖，矩阵中某个目标失败不会取消其他目标，但所选集合不完整时不发布 Release。

macOS 必须显式构建 `app,dmg`：只选 `dmg` 时 Tauri 会删除用于打包的临时 `.app`，不生成自动更新的 `.app.tar.gz` 和签名。选择目标时会提前验证此约束；每个桌面任务上传前还会调用 `scripts/release.py verify`，使用与最终汇总相同的规则检查安装包、更新包和签名，缺失时直接让对应构建任务失败。

桌面书源引擎需要附带 Java 21，因此桌面仅支持 x64/ARM64，不提供 Windows x86 或 Linux ARMv7 安装包。iOS 发版提供真机安装包；开发用模拟器产物不加入 Release。各桌面架构在对应的原生 runner 构建，JRE、JNI 和 Rust 架构一致。

Android 的 ABI 列表由 `android.abis` 配置。单独准备某个 ABI 的 AAR 时使用 `-Plegado.androidAbis=arm64-v8a`（替换成目标 ABI），再用 Tauri 的 `--target` 构建对应 APK。CI 会验证 APK 签名、实际 ABI，以及该 ABI 下的 Rust 和 QuickJS 动态库。

发版 APK 的 `versionName` 使用应用版本，`versionCode` 使用 `1000000 + GITHUB_RUN_NUMBER`，四个 ABI 与同次重跑保持一致。这个计数起点高于此前 `0.1.x` 的默认构建号，并让 RC → 正式版的构建号递增。普通 Package 构建仍使用源码中的默认构建号。

仓库 Actions Secrets 需要保留以下四项 Android 签名配置：

- `FOLIVER_ANDROID_KEYSTORE_BASE64`：PKCS12 keystore 的 Base64 内容。
- `FOLIVER_ANDROID_KEYSTORE_PASSWORD`：keystore 密码。
- `FOLIVER_ANDROID_KEY_ALIAS`：签名 alias。
- `FOLIVER_ANDROID_KEY_PASSWORD`：签名密钥密码。

Release 工作流在开始时检查这些配置和 `TAURI_SIGNING_PRIVATE_KEY` 是否齐全。只有最后汇总发布的 job 获得 `contents: write`。Windows/macOS 尚未配置系统证书签名，iOS IPA 需自行签名后安装，当前不自动提交应用商店。

所选任务全部成功后，CI 汇总并校验对应安装包及签名更新包，统一命名为 `Foliver-v版本-平台-架构.扩展名`，只额外生成应用自动更新所需的 `latest.json`。附件的 SHA-256 使用 GitHub 自带的 `digest`，发布前逐个与本地文件重新计算的哈希比较，不再上传独立校验文件或额外构建记录。缺少任意所选架构、安装包或更新签名即停止，不创建标签或 Release。

附件完整后创建标签和暂存草稿，生成更新说明，上传并校验附件数量、状态、大小以及 GitHub 返回的 SHA-256。全部通过后才公开发布；中途失败会保留草稿，重新运行失败的 job 可继续上传，保留编辑过的说明并清理过期附件。公开版本不会被修改。

只有高于全部现有稳定版本的新稳定版会成为 GitHub `latest`。预发布与历史稳定版不会替换当前稳定更新入口。发版工作流串行执行，所选目标中的任意构建失败即阻止发布，可直接在对应 job 查看错误。

流程参考 [Tauri 官方流水线](https://v2.tauri.app/distribute/pipelines/github/)、[Readest](https://github.com/readest/readest/blob/main/.github/workflows/release.yml) 与 [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev/blob/dev/.github/workflows/release.yml) 的版本驱动、矩阵构建和 Release 汇总方式，按本项目的 KMP 引擎及移动端构建补充处理。

## 应用自动更新

“我的 → 关于”提供应用名称、当前版本、系统与架构、GitHub 项目链接、问题反馈、启动时检查开关与手动检查入口。问题反馈打开 GitHub Issues 表单，预填版本与平台，由用户提交。页面使用仓库根目录 `LICENSE`、`THIRD_PARTY_NOTICES.txt`、`PRIVACY.txt` 展示离线可读的项目许可、主要第三方组件的许可原文与隐私说明；升级组件时同步更新第三方声明。正式构建默认仅在启动后后台检查一次；开发构建保留手动检查。关闭启动时检查后仍可手动操作。自动检查失败保持安静，手动检查和安装失败在界面显示具体错误。

所有平台只读取 GitHub 最新稳定 Release 的 `latest.json`，草稿和预发布不会推送到稳定客户端。客户端接受更高的正式版本，包含补丁版本。发现新版后显示说明，由用户决定是否下载和更新，不强制退出应用。选择“忽略此版本”后会保存该版本号，下次启动不再自动提示该版本；手动检查仍可查看和安装，后续新版本正常提示。“稍后”仅关闭本次弹窗。

- Windows：校验签名后启动 NSIS 更新程序，退出并由安装器重新启动。
- macOS：使用额外生成的 `.app.tar.gz` 更新应用，再由用户点击重启。
- Linux：分别为 AppImage、DEB 选择对应更新包；DEB 更新可能请求系统管理员授权。
- Android：选择当前 ABI 的 APK，校验签名、签名绑定版本、SHA-256、包名和版本号，再交给系统安装器。首次更新需要允许安装此来源应用；授权返回后点击“继续安装”，复用已验证的下载文件。
- iOS：显示更新并打开发布页；当前发行的是未签名 IPA，需自行签名安装，不能在应用内替换程序。

发版的桌面构建启用 [tauri.updater.conf.json](../src-tauri/tauri.updater.conf.json)，生成每个格式的更新文件及 `.sig`；Android APK 同时保留 Android 包签名和更新签名。更新签名与 Windows/macOS 的证书签名属于不同机制：发版构建不能传 `--no-sign`，该参数也会跳过更新签名。普通手动打包仍可不生成更新文件。

更新公钥在 `tauri.conf.json`；对应私钥保存在本地被 Git 忽略的 `.signing/updater.key`，CI 使用 Actions Secret `TAURI_SIGNING_PRIVATE_KEY`。当前密钥无密码，`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 可为空；若以后换用带密码的密钥，设置对应 Secret。私钥必须另外备份，后续版本保持使用同一密钥，避免已安装客户端无法验证更新。

更新清单只包含本次选择的目标；默认一级生成 6 个安装包、1 个 macOS 更新归档、5 个签名和 1 个 `latest.json`，共 13 个附件。选择全部三级时为 13 个安装包、2 个 macOS 更新归档、12 个签名和 1 个 `latest.json`，共 28 个附件。未包含在最新稳定版中的架构没有对应自动更新包，需要在发版时选择该级别。CI 在草稿生成后把更新说明写入 `latest.json`；手工修改草稿说明后，可重跑 Release 汇总 job 同步更新清单。是否公开由 `draft` 参数决定。
