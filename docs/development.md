# Foliver 开发与打包

工具版本与发布配置见 [GitHub Actions](../.github/workflows/build.yml)。CI 可手动选择全部平台、桌面、Android 或 iOS 构建。

## 各平台的运行时

| 平台 | 书源引擎 | 分发方式 |
|---|---|---|
| Windows、macOS、Linux | Kotlin/JVM + QuickJS JNI | 附带精简的 Java 21 运行时。 |
| Android | 系统 ART + QuickJS NDK 动态库 | 按 ABI 分发 Release APK。 |
| iOS | Kotlin/Native + QuickJS/mbedTLS 静态库 | 真机 IPA。 |

## 桌面

需要 Node.js 24、Rust stable、JDK 21、CMake 和 C/C++ 工具链。Linux 系统依赖见 CI。

```sh
npm ci
bash kotlin/gradlew -p kotlin --no-daemon --console=plain prepareDesktopJvmRuntime
npm run tauri -- dev
```

Windows 的 Gradle 命令使用 `.\kotlin\gradlew.bat -p kotlin --no-daemon --console=plain prepareDesktopJvmRuntime`。修改 Kotlin 后需重新准备 runtime。桌面打包使用 `npm run tauri -- build`；CI 分别生成 Windows NSIS、macOS DMG、Linux AppImage/DEB。

## Android

需要 Android SDK、NDK 与四个 Rust Android targets，最低 Android 8（API 26）。

```sh
bash kotlin/gradlew -p kotlin --no-daemon --console=plain stageTauriAndroidAar
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
bash kotlin/gradlew -p kotlin --no-daemon --console=plain -Plegado.iosNativeTargets=ios_arm64 :kmp-engine:linkReleaseFrameworkIosArm64
xcodebuild -create-xcframework -framework kotlin/kmp-engine/build/bin/iosArm64/releaseFramework/LegadoSourceEngine.framework -output src-tauri/plugins/source-engine/ios/Frameworks/LegadoSourceEngine.xcframework
npm ci
npm run tauri -- ios init --ci
npm run icons
npm run tauri -- ios build --no-sign --ci
```

重复组装 XCFramework 前需移走旧输出。模拟器开发可不传 `legado.iosNativeTargets`，使用 `:kmp-engine:linkReleaseFrameworkIosSimulatorArm64`。未签名 IPA 需签名后安装。
