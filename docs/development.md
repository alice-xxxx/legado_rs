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

只有新版本标签 `vX.Y.0` 触发发版，例如 `v0.2.0`、`v1.0.0`。普通提交、补丁标签（`v0.2.1`）和预发布标签均不触发。已经公开发布或小于已发布版本的标签会在构建前失败。失败后通过 Actions 的 Re-run failed jobs 重试；草稿可以更新，公开 Release 不覆盖。

用 Python 3.11+ 同步应用版本，提交后推送标签：

```sh
python scripts/release.py version 0.2.0
python scripts/release.py check v0.2.0
git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "chore: release v0.2.0"
git tag -a v0.2.0 -m "Foliver v0.2.0"
git push origin HEAD
git push origin v0.2.0
```

CI 校验标签与 npm、Tauri、Cargo 及两个锁文件的版本一致，并让所有构建使用同一个提交。发版固定构建以下全部支持架构：

| 平台 | 架构 | 安装包 |
|---|---|---|
| Windows | x64、ARM64 | 各一个 NSIS `.exe` |
| macOS | Intel x64、Apple Silicon ARM64 | 各一个 `.dmg` |
| Linux | x64、ARM64 | 各一个 `.AppImage` 和 `.deb` |
| Android | arm64-v8a、armeabi-v7a、x86_64、x86 | 各一个签名 `.apk` |
| iOS | ARM64 真机 | 一个未签名 `.ipa` |

桌面书源引擎需要附带 Java 21，因此桌面仅支持 x64/ARM64，不提供 Windows x86 或 Linux ARMv7 安装包。iOS 发版提供真机安装包；开发用模拟器产物不加入 Release。各桌面架构在对应的原生 runner 构建，JRE、JNI 和 Rust 架构一致。

Android 的 ABI 列表由 `android.abis` 配置。单独准备某个 ABI 的 AAR 时使用 `-Plegado.androidAbis=arm64-v8a`（替换成目标 ABI），再用 Tauri 的 `--target` 构建对应 APK。CI 会验证 APK 签名、实际 ABI，以及该 ABI 下的 Rust 和 QuickJS 动态库。

仓库 Actions Secrets 需要保留以下四项 Android 签名配置：

- `FOLIVER_ANDROID_KEYSTORE_BASE64`：PKCS12 keystore 的 Base64 内容。
- `FOLIVER_ANDROID_KEYSTORE_PASSWORD`：keystore 密码。
- `FOLIVER_ANDROID_KEY_ALIAS`：签名 alias。
- `FOLIVER_ANDROID_KEY_PASSWORD`：签名密钥密码。

Release 工作流在开始时检查这些配置是否齐全。只有最后汇总 Release 的 job 获得 `contents: write`；构建 job 保持只读权限。Windows/macOS 沿用未签名分发，iOS IPA 需自行签名后安装，当前不自动提交应用商店。

全部 11 个构建任务成功后，CI 汇总并校验 13 个安装包及签名更新包，统一命名为 `Foliver-vX.Y.0-平台-架构.扩展名`，同时生成 `SHA256SUMS`、`release-manifest.json` 和 `latest.json`。manifest 包含源码提交、构建链接、文件大小与 SHA-256；校验文件也覆盖两个 manifest 和签名文件。缺少任意架构、安装包或更新签名即停止，不创建新的草稿。

最后创建 GitHub Release 草稿、自动生成更新说明、上传安装包，并核对上传状态和大小。已有草稿重跑时更新同名附件，保留手工编辑的说明。打开 Actions 汇总中的草稿链接检查，再手动点击 Publish release；CI 不自动公开发布。

## 应用自动更新

“我的 → 关于”提供应用名称、当前版本、系统与架构、GitHub 项目链接、问题反馈、启动时检查开关与手动检查入口。问题反馈打开 GitHub Issues 表单，预填版本与平台，由用户提交。页面使用仓库根目录 `LICENSE`、`THIRD_PARTY_NOTICES.txt`、`PRIVACY.txt` 展示离线可读的项目许可、主要第三方组件的许可原文与隐私说明；升级组件时同步更新第三方声明。正式构建默认仅在启动后后台检查一次；开发构建保留手动检查。关闭启动时检查后仍可手动操作。自动检查失败保持安静，手动检查和安装失败在界面显示具体错误。

所有平台只读取已公开 Release 的 `latest.json`，草稿不会推送到客户端。客户端只接受更高的 `X.Y.0` 正式版本。发现新版后显示说明，由用户决定是否下载和更新，不强制退出应用。选择“忽略此版本”后会保存该版本号，下次启动不再自动提示该版本；手动检查仍可查看和安装，后续新版本正常提示。“稍后”仅关闭本次弹窗。

- Windows：校验签名后启动 NSIS 更新程序，退出并由安装器重新启动。
- macOS：使用额外生成的 `.app.tar.gz` 更新应用，再由用户点击重启。
- Linux：分别为 AppImage、DEB 选择对应更新包；DEB 更新可能请求系统管理员授权。
- Android：选择当前 ABI 的 APK，校验签名、签名绑定版本、SHA-256、包名和版本号，再交给系统安装器。首次更新需要允许安装此来源应用；授权返回后点击“继续安装”，复用已验证的下载文件。
- iOS：显示更新并打开发布页；当前发行的是未签名 IPA，需自行签名安装，不能在应用内替换程序。

发版的桌面构建启用 [tauri.updater.conf.json](../src-tauri/tauri.updater.conf.json)，生成每个格式的更新文件及 `.sig`；Android APK 同时保留 Android 包签名和更新签名。更新签名与 Windows/macOS 的证书签名属于不同机制：发版构建不能传 `--no-sign`，该参数也会跳过更新签名。普通手动打包仍可不生成更新文件。

更新公钥在 `tauri.conf.json`；对应私钥保存在本地被 Git 忽略的 `.signing/updater.key`，CI 使用 Actions Secret `TAURI_SIGNING_PRIVATE_KEY`。当前密钥无密码，`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 可为空；若以后换用带密码的密钥，设置对应 Secret。私钥必须另外备份，后续版本保持使用同一密钥，避免已安装客户端无法验证更新。

更新清单含 12 个目标：Windows、macOS 各两个，Linux 两种格式各两个，Android 四个。共上传 30 个附件：13 个安装包、2 个 macOS 更新归档、12 个签名和 3 个清单/校验文件。CI 在草稿生成后把更新说明写入 `latest.json`，再重新计算校验值；手工修改草稿说明后，可重跑 Release 汇总 job 同步更新清单。公开发布仍由人工完成。
