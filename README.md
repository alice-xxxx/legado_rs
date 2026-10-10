# legado_rs

基于 Vue、Tauri 和 Rust 的阅读应用，保留 Kotlin Multiplatform 书源引擎。目标是 Android、iOS、Windows、macOS 和 Linux。

## 开发

需要 Node.js 24、Rust stable、JDK 21，以及 CMake/C/C++ 工具链。实际桌面应用另需对应平台的 Tauri 系统依赖。

```sh
npm ci
npm run build
./kotlin/gradlew -p kotlin --no-daemon --console=plain prepareDesktopJvmRuntime
```

## 文档

- [架构与产品约定](docs/architecture.md)：职责边界、存储格式和实现原则。
- [开发说明](docs/development.md)：环境、平台构建和 CI。

