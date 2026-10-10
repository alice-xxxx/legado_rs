# Tauri 书源解析插件

插件把 Rust 的 `execute_source_engine` 调用转给 Android Kotlin 或 iOS Swift 原生入口。平台入口再调用独立 KMP 解析器；解析器需要的 HTTP 和存储通过 Android JNI 或 iOS C ABI 回到 Rust。

Android 构建先运行 Kotlin Gradle 的 `stageTauriAndroidAar`；iOS 构建先生成 `Frameworks/LegadoSourceEngine.xcframework`。这些文件是构建产物，已由本目录 `.gitignore` 排除。
