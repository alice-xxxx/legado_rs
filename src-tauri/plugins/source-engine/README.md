# source-engine 插件

全平台书源引擎 SDK。App 注册 `tauri_plugin_source_engine::init()` 后，通过 `SourceEngineExt::source_engine()` 调用具名、类型化 API：

- `search`、`explore_kinds`、`explore`：搜索与发现。
- `detail`、`chapters`、`content`：详情、目录和正文。
- `resolve_media`、`http_tts_audio`、`login`：媒体请求、在线朗读音频与源登录。

网页登录同样通过 `SourceEngineExt::source_engine()` 调用 `start_web_login`、`complete_web_login`、`cancel_web_login`。所有平台共用请求、捕获结果和取消状态类型；App 负责校验书源版本及恢复代次，再提交 Cookie。桌面端在 App setup 中调用 `install_browser_app(app.handle().clone())` 绑定浏览器宿主。

请求、结果及书籍模型由插件统一导出。`EngineContext` 注入数据目录、取消信号和 `StorageHost`。App 通过类型化的 `get`、`put`、`remove` 提供持久化；`Client` 实现 `SourceExecutor`，应用不再编写一套委托实现。

| Rust 模块 | 职责 |
|---|---|
| `api` | 业务请求、结果和宿主存储端口。 |
| `engine` | 具名 API、单次调用的取消和生命周期。 |
| `http` | 请求、重定向、超时、取消和 Cookie Jar。 |
| `storage` | 原生存储参数转换和 Cookie Jar 失效。 |
| `image` | 有界解码、编码、裁剪、旋转、切分和拼接。 |
| `browser` | 浏览器输入、结果及会话归属检查。 |
| `desktop` | 共用 JVM/JNI 和桌面 WebView。 |
| `windows`、`macos`、`linux`、`android`、`ios` | 系统初始化及各自的原生边界；移动端 FFI 位于所属平台目录。 |

KMP 的 `api/ReaderSource.kt` 和 `api/Login.kt` 直接实现业务能力；`host/` 分别对接 HTTP、存储、图像、浏览器和文件目录。跨语言 JSON 只存在于私有编解码边界，没有按操作名分发业务的总入口。

| 平台 | 执行路径 |
|---|---|
| Windows、macOS、Linux | 插件内 JVM/JNI 调用 Kotlin 类型化接口，附带精简 JRE；也可通过 `JvmEngine` 独立调用。 |
| Android | Tauri Java 插件调用 KMP AAR，宿主回调通过 JNI 进入 Rust。 |
| iOS | Tauri Swift 插件调用 KMP XCFramework，宿主回调通过 C ABI 进入 Rust。 |

`engine/` 保留独立 Gradle 构建、规则与脚本引擎；不包含上游应用数据库、阅读器或播放器。原始包名及源协议保留以兼容脚本。HTTP、浏览器、图像、存储桥接和原生头文件均归本插件。

KMP 代码按职责分层：

| 源集 | 职责 |
|---|---|
| `commonMain` | 规则解析、脚本协议、数据模型、文件与网络兼容逻辑，以及宿主端口和统一初始化。 |
| `jvmAndAndroidMain` | 共用的 QuickJS、JVM 加密和标准库适配。 |
| `jvmMain`、`androidMain` | JVM 嵌入、Android JNI 和各自的系统适配。 |
| `nativeMain`、`iosMain` | Native JS/加密、C ABI 及 iOS 系统适配。 |

目录解析不查询应用数据库、不修改阅读进度；正文解析不写文件或保存章节。源脚本的缓存、Cookie 和大变量只依赖宿主存储接口。剪贴板等交互交给宿主 UI 端口。

KMP 核心通过 `SourceStorage` 访问存储，通过 `AppFilesDir` 获取当前调用的私有目录。目录提供器与文件 API 在公共层使用 Okio，包含路径校验、流式下载和文件管理；核心不推断便携模式、备份目录或原应用的外部存储。HTTP 不再构造 Ktor/Cronet 客户端，也不维护图片下载进度监听。

脚本加密的文本、字节流及 Hex/Base64 转换共用公共实现，平台只提供原始字节算法。HTTP 表单编码、multipart 参数处理、解压判断和 URL 查询参数也在公共层维护；JVM 保留 OkHttp/JDK 适配，Native 仅保留库缺失的兼容编码及算法后端。

脚本绑定容器、求值状态恢复、字符串字面量和类名匹配也在公共层维护；JVM 的作用域、编译结果和对象直接实现公共接口。参数数值转换与重载评分由反射和生成分派共用，JSON 解析及反射异常包装留在 JVM 适配。Native 的 JS 对象包装和生成方法注入使用公共实现，平台桥仅负责值与句柄传递。宿主 HTTP 的数据模型、构建器、请求转换和回调使用公共实现，iOS 通过类型别名接入。Cookie、日期模式、字数、拼音排序及带锁 Map 的后备实现共用公共代码，并保留原有平台差异。

源脚本的登录、变量编辑和验证码接口保留，交互实现由 `VerificationUiProvider` 注入并直接返回结果。未提供交互能力时明确报错，不再发送无人消费的应用事件或轮询缓存等待。普通登录表单、菜单网页登录仍走 SDK 的具名登录 API。

Android 浏览器使用 AndroidX WebKit 的独立 profile，需要系统 WebView 支持 `MULTI_PROFILE`；不支持时明确提示更新系统 WebView。iOS 使用非持久化 `WKWebsiteDataStore`，桌面使用私有 WebView。普通 HTTP 规则不依赖浏览器能力。

工具链版本统一维护在 [toolchain.properties](toolchain.properties)。构建任务、运行时准备与打包步骤见[开发说明](../../../docs/development.md)；应用运行时不自动编译引擎或寻找系统 JDK。
