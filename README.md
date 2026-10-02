# Legado 书源解析核心

本仓库提供可独立构建的 Kotlin Multiplatform 书源解析器和 Rust 宿主。Kotlin 执行 `WebBook`、`AnalyzeUrlCore`、`AnalyzeRuleCore` 等规则并解析响应；Rust 负责 HTTP、Cookie、代理、重定向和持久化存储。构建输入和源码来源见 [`kotlin/kmp-engine/SOURCE_MANIFEST.txt`](kotlin/kmp-engine/SOURCE_MANIFEST.txt)。

## 平台调用路径

| 平台 | 解析与宿主调用 |
| --- | --- |
| Windows、Linux、macOS | 测试界面 → Tauri command → Rust → JNI/JVM → KMP；KMP 的网络与存储请求回到 Rust |
| Android | `AndroidSourceEngine.execute` → KMP → JNI → Rust |
| iOS | Swift → `IosSourceEngine.execute` → Kotlin/Native → cinterop → Rust C ABI |

Android AAR 包含 Rust 与 QuickJS 的 `arm64-v8a`、`armeabi-v7a`、`x86`、`x86_64` 原生库。iOS framework 需要与对应架构的 Rust staticlib 一起链接。桌面测试界面只提交操作请求，解析、HTTP 和存储由上述 Rust/KMP 链路执行。

## 构建

桌面需要 JDK 21、Rust stable、Node.js 24、CMake 和 C/C++ 编译器；Linux 另需 Tauri 的 GTK/WebKitGTK 开发包。Android 另需 Android SDK、NDK 和 Rust Android targets。iOS 需要 macOS、Xcode 和 Rust Apple targets。

```sh
npm ci
npm run build
```

```sh
cd kotlin
./gradlew --no-daemon --console=plain prepareDesktopJvmRuntime
./gradlew --no-daemon --console=plain :kmp-engine:bundleAndroidMainAar
```

iOS 在 macOS 上构建设备和模拟器 framework：

```sh
cd kotlin
./gradlew --no-daemon --console=plain \
  :kmp-engine:linkReleaseFrameworkIosArm64 \
  :kmp-engine:linkReleaseFrameworkIosSimulatorArm64
```

GitHub Actions 在 Windows、Linux、macOS 检查前端与 Rust，并构建 KMP JVM/QuickJS；另构建 Android AAR 和 iOS framework。当前尚未接入 JS `image.*` 的图片像素处理；依赖图片解密、切片、拼接或旋转的规则尚不可用。桌面安装包仍需集成 JVM 和 QuickJS 运行库。
