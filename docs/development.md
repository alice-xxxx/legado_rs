# Foliver 开发与打包

JDK、Kotlin、KSP、Gradle、AGP 与 Android SDK/NDK 的版本统一维护在 [toolchain.properties](../src-tauri/plugins/source-engine/toolchain.properties)，App、插件和 KMP 使用同一套配置。修改版本后运行 `node src-tauri/toolchain.mjs --sync`，同步 KMP 的 Gradle wrapper 与 daemon JDK；已有 Android 工程再运行 `node src-tauri/android/configure.mjs`。CI 自动执行。发布配置见 [GitHub Actions](../.github/workflows/build.yml)。

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
