# 开发与构建

从仓库根目录执行。工具版本和平台打包参数以 [CI 配置](../.github/workflows/build.yml) 为准。

## 环境

- Node.js 24、Rust stable、JDK 21、CMake 和 C/C++ 工具链。
- 桌面需要对应平台的 Tauri 系统依赖；Linux 依赖安装清单见 CI。
- Android 8+（API 26），需要 Android SDK、NDK 和对应 Rust targets。
- iOS 需要 macOS、Xcode 和对应 Rust targets。

## 桌面开发

```sh
npm ci
npm run build
./kotlin/gradlew -p kotlin --no-daemon --console=plain prepareDesktopJvmRuntime
cargo check --locked --manifest-path src-tauri/Cargo.toml --lib
npm run tauri -- dev
```

Windows 将 Gradle 命令替换为：

```powershell
.\kotlin\gradlew.bat -p kotlin --no-daemon --console=plain prepareDesktopJvmRuntime
```

修改 Kotlin 后重新准备 runtime。桌面打包同样先准备 runtime，再执行 `npm run tauri -- build`；runtime 包含 JVM、引擎 JAR 及 QuickJS 库。

## Android

```sh
./kotlin/gradlew -p kotlin --no-daemon --console=plain stageTauriAndroidAar
npm run tauri -- android init --ci
node scripts/configure-tauri-android.mjs
npm run tauri -- android build --debug --apk --ci
```

Windows 使用 `gradlew.bat`。原生生成项目、AAR 和打包产物由构建流程生成。

## iOS

在 macOS 执行：

```sh
export IPHONEOS_DEPLOYMENT_TARGET=15.0
bash scripts/build-ios-frameworks.sh build-frameworks
bash scripts/build-ios-frameworks.sh package-xcframework
npm run tauri -- ios init --ci
npm run tauri -- ios build --no-sign --ci
```

构建流程生成 KMP XCFramework、QuickJS/mbedTLS 静态库，并由 source-engine 插件接入最终链接。未签名设备包需签名后安装。

## 检查与 CI

- `npm run build`：前端类型检查和构建。
- `cargo check --locked --manifest-path src-tauri/Cargo.toml --lib`：当前平台的 Rust 桌面检查。
- `node scripts/verify-platforms.mjs --artifacts`：工具与产物检查。

移动音频使用 [native-audio 1.0.5](https://github.com/uvarov-frontend/tauri-plugin-native-audio)，系统朗读使用 [tts 0.2.1](https://github.com/brenogonzaga/tauri-plugin-tts)。iOS 模板声明后台音频和本地资源访问；Android 生成脚本配置 API 26 和 HLS 支持。Android 系统 TTS 暂停后，继续时从当前片段重读。

当前 CI 仅由 `workflow_dispatch` 手动触发，包含三个桌面平台、Android 和 iOS 构建。编译或归档检查不代表设备上的阅读、播放、登录及系统唤起验证。
