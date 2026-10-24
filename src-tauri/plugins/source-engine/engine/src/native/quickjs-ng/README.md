# QuickJS-NG 源码

源码来自 [quickjs-ng/quickjs](https://github.com/quickjs-ng/quickjs)，许可见 [LICENSE](LICENSE)。头文件中的版本宏不提供具体上游提交信息。

本项目的编译入口：

- 桌面 JNI：`src-tauri/plugins/source-engine/engine/build.gradle.kts` 和 `src-tauri/plugins/source-engine/engine/src/native/cpp/CMakeLists.txt`。
- Android JNI：`src-tauri/plugins/source-engine/engine/kmp-engine/build.gradle.kts` 使用同一 CMake 工程。
- iOS 静态库：`src-tauri/plugins/source-engine/engine/kmp-engine/scripts/build-ios-native.sh`。

编译的核心 C 文件为 `quickjs.c`、`dtoa.c`、`libregexp.c`、`libunicode.c`；Kotlin/Native 绑定位于 `src-tauri/plugins/source-engine/engine/kmp-engine/src/nativeInterop/cinterop/quickjs.def`。

升级时记录准确的上游提交、检查绑定 API 和编译文件列表，并验证各受影响平台。本仓库没有独立的 QuickJS 同步 workflow；平台封装和编译选项分别在绑定、JNI wrapper 与构建文件中维护。
