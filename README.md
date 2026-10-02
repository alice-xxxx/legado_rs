# Legado 书源解析核心

本仓库包含独立的 Kotlin Multiplatform 书源解析器和 Rust 宿主。Kotlin 执行 `WebBook`、`AnalyzeUrlCore`、`AnalyzeRuleCore` 等规则；Rust 执行 HTTP、Cookie、代理、重定向和持久化存储。源码来源见 [`kotlin/kmp-engine/SOURCE_MANIFEST.txt`](kotlin/kmp-engine/SOURCE_MANIFEST.txt)。

## 平台调用

| 平台 | 解析器入口与宿主 |
| --- | --- |
| Windows、Linux、macOS | Tauri command → Rust → JNI/JVM → KMP；安装包携带 JVM、依赖和 QuickJS |
| Android | Tauri command → Rust mobile plugin → Android KMP AAR；KMP 通过 JNI 调用 Rust Host |
| iOS | Tauri command → Rust mobile plugin → Swift → Kotlin/Native framework；KMP 通过 C ABI 调用 Rust Host |

移动端原生插件和 KMP framework/AAR 由构建任务生成，不提交编译产物。移动端解析请求、HTTP 和存储都经过 Rust 宿主。

## 构建

桌面需要 JDK 21、Rust stable、Node.js 24、CMake 和 C/C++ 编译器；Linux 另需 Tauri 的 GTK/WebKitGTK 开发包。

```sh
npm ci
npm run build
cd kotlin
./gradlew --no-daemon --console=plain prepareDesktopJvmRuntime
```

Android 另需 Android SDK、NDK 和 Rust Android targets：

```sh
cd kotlin
./gradlew --no-daemon --console=plain stageTauriAndroidAar
cd ..
npm run tauri -- android init --ci
node scripts/configure-tauri-android.mjs
npm run tauri -- android build --debug --apk --ci
```

iOS 需要 macOS、Xcode 和 Rust Apple targets。构建设备与模拟器 framework 后，将其合并为插件所用 XCFramework：

```sh
cd kotlin
./gradlew --no-daemon --console=plain \
  :kmp-engine:linkReleaseFrameworkIosArm64 \
  :kmp-engine:linkReleaseFrameworkIosSimulatorArm64
cd ..
mkdir -p src-tauri/plugins/source-engine/ios/Frameworks
xcodebuild -create-xcframework \
  -framework kotlin/kmp-engine/build/bin/iosArm64/releaseFramework/LegadoSourceEngine.framework \
  -framework kotlin/kmp-engine/build/bin/iosSimulatorArm64/releaseFramework/LegadoSourceEngine.framework \
  -output src-tauri/plugins/source-engine/ios/Frameworks/LegadoSourceEngine.xcframework
```

GitHub Actions 为 Windows、Linux、macOS、Android 和 iOS 构建测试安装包。当前尚未接入 JS `image.*` 的图片像素处理；依赖图片解密、切片、拼接或旋转的规则尚不可用。
