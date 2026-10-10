# 开发说明

从仓库根目录运行以下命令。平台版本和打包步骤以 [CI workflow](../.github/workflows/build.yml) 为准，避免在多处维护重复的 SDK 清单。

## 环境

需要 Node.js 24、Rust stable、JDK 21、CMake 和 C/C++ 编译器。桌面安装包携带 JVM、依赖和 QuickJS；移动端使用 KMP 原生入口，不打包桌面 JVM。

当前云环境的工具已安装，每个 shell 先激活：

```sh
source /workspace/.setup/activate.sh
```

实际 Linux Tauri 构建或运行还需 GTK 3、WebKitGTK 4.1 等系统依赖。本云环境已有可信 sysroot，使用：

```sh
source /workspace/.setup/native-env.sh
```

macOS/iOS 构建需要 macOS 和 Xcode。Android 需要 workflow 所列 SDK、NDK、CMake 和四个 ABI 的 Rust targets。Linux 云环境不能执行 iOS 构建；没有 KVM 也不能把安装了 Android AVD 当作运行验证。

## 安装与检查

```sh
npm ci
npm run build
./kotlin/gradlew -p kotlin --no-daemon --console=plain prepareDesktopJvmRuntime
```

检查 Rust 桌面目标并运行真实 Tauri 应用：

```sh
cargo check --locked --manifest-path src-tauri/Cargo.toml --lib -j2
npm run tauri -- dev
```

桌面 `tauri dev` 的热重载只编译当前桌面目标，不能发现 Android Kotlin、iOS Swift/Kotlin Native 或另一桌面平台的编译错误；跨平台结果以对应 CI job 为准。桌面应用使用已准备的 KMP runtime，修改 Kotlin 后需要重新准备 runtime 才能运行新引擎代码。

## 平台打包

桌面：先准备上面的 JVM runtime，再执行 `npm run tauri -- build`。Linux 原生检查须激活系统依赖；Windows/macOS 使用各自平台工具链。

Android：

```sh
./kotlin/gradlew -p kotlin --no-daemon --console=plain stageTauriAndroidAar
npm run tauri -- android init --ci
node scripts/configure-tauri-android.mjs
npm run tauri -- android build --debug --apk --ci
```

iOS，在 macOS 上运行：

```sh
export IPHONEOS_DEPLOYMENT_TARGET=15.0
./scripts/build-ios-frameworks.sh build-frameworks
./scripts/build-ios-frameworks.sh package-xcframework
npm run tauri -- ios init --ci
npm run tauri -- ios build --no-sign --ci
```

以上与 CI 的设备 IPA 命令一致。模拟器构建使用 `npm run tauri -- ios build --target aarch64-sim --no-sign --ci`。设备 IPA 需重签后安装。Android/iOS 原生生成项目、AAR、XCFramework 和安装包均不提交。

## CI 与体积

- 主 workflow 根据应用构建输入触发五个平台 job；仅改文档不重编全部安装包。
- iOS framework 使用工具链和实际引擎构建输入的精确缓存；命中后跳过重复构建/打包。
- Gradle 使用 basic 开源缓存。桌面使用裁剪 JVM/runtime，保留所需类、资源和模块；Android 保留四个 ABI。进一步裁剪须验证实际引擎和安装包，不能靠删必要功能减小体积。
- 检查安装包可运行 `node scripts/verify-platforms.mjs --artifacts`；它检查产物存在、Android ABI 内容及 APK/IPA 归档完整性，不验签或链接符号，也不代表应用功能可用。