# source-engine 插件

全平台书源引擎 SDK。App 注册 `tauri_plugin_source_engine::init()` 后，通过 `SourceEngineExt::source_engine()` 调用具名、类型化 API：

- `search`、`explore_kinds`、`explore`：搜索与发现。
- `detail`、`chapters`、`content`：详情、目录和正文。
- `resolve_media`、`http_tts_audio`、`login`：媒体请求、在线朗读音频与源登录。

网页登录同样通过 `SourceEngineExt::source_engine()` 调用 `start_web_login`、`complete_web_login`、`cancel_web_login`。所有平台共用请求、捕获结果和取消状态类型；App 负责校验书源版本及恢复代次，再提交 Cookie。桌面端在 App setup 中调用 `install_browser_app(app.handle().clone())` 绑定浏览器宿主。

请求、结果及书籍模型由插件统一导出。`ExecutionContext` 注入数据目录、取消信号和 `StorageHost`。App 通过 `get`、`put`、`remove` 实现持久化，命名空间和删除条件均有明确类型；原生请求解码、执行租约和 Cookie Jar 失效由插件处理。App 不接收 JNI/C ABI 请求结构，插件不向 WebView 暴露通用执行命令。

| 平台 | 执行路径 |
|---|---|
| Windows、macOS、Linux | 插件内 JVM/JNI 调用 Kotlin 类型化接口，附带精简 JRE；也可通过 `JvmEngine` 独立调用。 |
| Android | Tauri Java 插件调用 KMP AAR，宿主回调通过 JNI 进入 Rust。 |
| iOS | Tauri Swift 插件调用 KMP XCFramework，宿主回调通过 C ABI 进入 Rust。 |

`engine/` 保留独立 Gradle 构建、规则与脚本引擎；不包含上游应用数据库、阅读器或播放器。原始包名及源协议保留以兼容脚本。HTTP、浏览器、图像、存储桥接和原生头文件均归本插件。

KMP 代码按职责分层：

| 源集 | 职责 |
|---|---|
| `commonMain` | 规则解析、脚本协议、数据模型，以及按执行上下文访问的宿主端口。 |
| `jvmAndAndroidMain` | 共用的 QuickJS、JVM 加密和标准库适配，统一初始化。 |
| `jvmMain`、`androidMain` | JVM 嵌入、Android JNI 和各自的系统适配。 |
| `nativeMain`、`iosMain` | Native JS/加密、C ABI 及 iOS 系统适配。 |

目录解析不查询应用数据库、不修改阅读进度；正文解析不写文件或保存章节。源脚本的缓存、Cookie 和大变量只依赖宿主存储接口。剪贴板等交互交给宿主 UI 端口。

工具链版本统一维护在 [toolchain.properties](toolchain.properties)。构建任务、运行时准备与打包步骤见[开发说明](../../../docs/development.md)；应用运行时不自动编译引擎或寻找系统 JDK。
