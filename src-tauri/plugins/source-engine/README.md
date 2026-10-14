# source-engine 插件

移动端书源引擎适配器：

- Android：Tauri Kotlin 插件调用 KMP AAR，宿主 HTTP 和存储通过 JNI 进入 Rust。
- iOS：Swift 插件调用 KMP XCFramework，宿主能力通过 C ABI 进入 Rust。
- 桌面：应用通过 Rust 的 JVM/JNI 执行器调用引擎；插件的 desktop.execute 返回 UnsupportedTarget。

构建前置任务与打包步骤见[开发说明](../../../docs/development.md)。Android 的 `stageTauriAndroidAar` 生成插件 AAR；iOS 流程生成 `ios/Frameworks/LegadoSourceEngine.xcframework` 和 native 静态库，插件 build.rs 校验并链接这些产物。

权限定义位于 `permissions/`，自动生成的参考文件不作为手工维护入口。
