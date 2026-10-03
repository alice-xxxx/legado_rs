# 书源隐藏浏览器宿主

状态：设计与缺口记录，尚未实现 `ContentRule.webJs` 宿主执行。当前 KMP 正文请求成功只证明普通 HTTP 和正文规则可用，不证明隐藏页面脚本执行。

## 已核实的行为

用户提供的 `伪速读谷²` 书源在 `ruleContent.webJs` 中使用 `document.querySelector`、`DOMParser`，并以 `XMLHttpRequest.open("GET", url, false)` 同步请求后续分页，再把 `.con` 内容追加到当前 DOM。它最多循环 20 页。

新项目保存了 `ContentRule.webJs`、`sourceRegex` 和 `replaceRegex`。但 `WebBook.getContentAwait` 将响应 body 直接传给 `BookContent.analyzeContent`；正文流程只解析正文规则和 `nextContentUrl`，没有调用 `ContentRule.webJs` 或 `ContentRule.sourceRegex`。旧 legado 的对应 `WebBook` / `BookContent` 调用链也没有调用这两个字段。`replaceRegex` 是另一个独立正文替换字段。

因此该真实书源的 `nextContentUrl` 可以单独产生分页链接，但不能据此说 `webJs` 得到支持。该书源当前 `sourceRegex` 的值是 `[^<]*首发更新站点。<br />\\s*`，看起来像移除广告文本的正文正则；`replaceRegex` 为空。不得把它偷偷解释为正文替换规则，也不得因此对普通章节开启资源嗅探。字段语义与用户数据疑似不匹配，应保留原值并在规则编辑/迁移设计中明确呈现。

### `sourceRegex` 语义

旧规则 UI 将该字段标为“资源正则（sourceRegex）”。旧 `BackstageWebViewFactory` 注释及桌面实现将它用于资源 URL 嗅探：浏览器加载页面时观察子资源 URL，正则命中后将该资源 URL 作为此次 WebView 响应结果。它不是 HTML 正文的正则替换。桌面 legacy 实现仅在配置了 `sourceRegex` 或 `overrideUrlRegex` 时进入嗅探路径，避免一般页面等待一个根本不会出现的资源正则。

新宿主应区分两个互不隐式触发的操作：

1. `TransformContentPage`：执行 `ContentRule.webJs`，返回处理后的 DOM/HTML，之后照常运行正文 selector。此操作不因 `ContentRule.sourceRegex` 非空而进入嗅探。
2. `SniffResource`：仅由既有显式 WebView API（例如 `getStrResponse(..., sourceRegex)` / `webViewGetSource`）触发，观察网络资源请求并在正则命中时返回 URL。`overrideUrlRegex` 是另一种导航 URL 嗅探，应继续分开。

当 `ContentRule.sourceRegex` 有值但正文操作没有显式嗅探调用时，不得等待嗅探命中，也不得把值套用到 HTML。对 `伪速读谷²` 的广告清理，需让用户之后明确修改成独立的 `replaceRegex` 规则；自动迁移可能改变其原意，本设计不自动改写书源。

## KMP 与平台宿主现状

KMP 的 `BackstageWebViewFactory` 是一个既有 common 接口，`AnalyzeUrlCore` 在 `UrlOption.useWebView` 分支和 `java.webView*` 扩展里使用 `BackstageWebViewProviders.get()`。factory 未注册会直接报错。这是独立于正文 `ContentRule.webJs` 未调用的第二个缺口。

| 平台 | 当前新项目状态 | 具体限制 |
| --- | --- | --- |
| 桌面/JVM | `RustSourceEngineProviders.install()` 未注册 BackstageWebView factory；没有新项目的桌面浏览器实现。 | Tauri 隐藏 WebView 必须由 Rust/Tauri 生命周期管理，并在 UI 线程创建和销毁。书源执行目前会在 JNI / `spawn_blocking` 工作线程同步运行；不能从该线程阻塞等待一次必须由 UI 线程驱动的调用。 |
| Android | `AndroidSourceEngineRuntime.install()` 未注册 factory；提取后的 KMP 无 Android WebView 实现。旧 legado 有隐藏 Android WebView，可参考其 DOM 执行与生命周期。 | `shouldInterceptRequest` 可以拦截 HTTP(S) 子请求，但会在 WebView 的请求线程回调，且看不到请求 body；不能把所有方法都当作可透明代理的 GET。执行 `evaluateJavascript`、创建和销毁 WebView 必须切到主 Looper，但不能阻塞主线程等 Rust 网络完成。 |
| iOS | 有 `IosBackstageWebView.kt` / `WKWebView` 实现与 `registerIosBackstageWebView()`，但 `IosSourceEngineRuntime.install()` 没有调用注册函数。 | `WKURLSchemeHandler` 只能处理自定义 scheme，不能接管 WKWebView 的内建 `http` / `https` scheme。`WKNavigationDelegate` 也不是任意子资源/ XHR 响应体拦截器。需验证自定义 scheme 上同步 XHR 是否可工作，不能假设通过 URLSchemeHandler 就自动支持同步 XHR。现有 iOS 实现还明确拒绝 `sourceRegex` 嗅探。 |

## 目标调用接口与生命周期

不应把源脚本交给 Vue 阅读 WebView。书源引擎只接收浏览器处理后的结果。建议把下面的能力作为 `SourceEngineHost` 的显式、挂起式子能力，并在当前 source-engine call 的 host scope 内调用；不要增加进程级“当前浏览器请求”全局变量。

```kotlin
data class SourceBrowserRequest(
    val html: String,
    val baseUrl: String,
    val mode: SourceBrowserMode,
    val javaScript: String? = null,
    val sourceRegex: String? = null,
    val overrideUrlRegex: String? = null,
    val delayTimeMs: Long = 0,
)

sealed interface SourceBrowserMode {
    data object TransformContentPage : SourceBrowserMode
    data object ReturnPageSource : SourceBrowserMode
    data object SniffResource : SourceBrowserMode
}

data class SourceBrowserResult(
    val finalUrl: String,
    val htmlOrResourceUrl: String,
    val matchedBy: String? = null,
)

interface SourceBrowserHost {
    suspend fun execute(request: SourceBrowserRequest): SourceBrowserResult
}
```

`SourceBrowserRequest` 不带 cookie、登录头、Rust capability token 或持久化资源 URL。host 从当前 `SourceEngineHost` scope 建立一次性 Rust 网络会话；该会话绑定本次书源、原始 base URL、默认 headers/cookies、书源代理、TLS 选项、超时及 redirect 策略。必须在返回、失败、超时、协程取消时销毁浏览器并撤销会话。HTML、脚本、浏览器 DOM 与 capability token 只存在于引擎/隐藏宿主生命周期中，不写入书架 JSON，也不返回 Vue。

实现上可以演进既有 `BackstageWebViewFactory` / handle，而不是再造另一个可见 WebView API；但需要将普通 HTML transform 与显式 URL/resource sniff 作为不同的类型化模式，以免复用旧的“带有 regex 就一直等命中”逻辑。

## Rust 网络会话与代理

隐藏浏览器不得自行直连书源站点。Rust 为每次执行创建短时、不可猜测的 session capability，在 loopback-only 的内部代理路由注册；只绑定 `127.0.0.1` / `::1`，每个请求校验 token、会话存活、URL scheme 为 HTTP(S)、方法/大小/超时上限。路由不是通用公开代理，不允许任意客户端调用，不开放目录、存储写入或任意文件读取。用 Rust 当前的 reqwest source HTTP 执行路径传递每次请求，并应用该调用的 headers、CookieJar、书源代理、TLS 与 redirect 行为。敏感请求头只在 Rust session 内；不注入浏览器脚本、不返回 Cookie/Set-Cookie 到 DOM。

浏览器中的 XHR/fetch 与加载的外部资源须明确路由到这个 session proxy。代理响应要保留浏览器解析所需的状态码、MIME、长度及安全的响应头；请求取消、页销毁或超时应取消上游 reqwest。不得仅把初始 HTML 通过 Rust 下载，就声称后续脚本网络也由 Rust 处理。

更可靠的跨端接口是 native adapter 把浏览器请求事件映射成上述 capability session 的请求/响应；但同步 XHR 会阻塞 JS 执行线程，所以 adapter 必须能同步交付请求结果或提供经真实 WebView 验证的同步桥，不能在等待浏览器返回时占住浏览器自己的 UI/event-loop 线程。Rust 上游执行仍应在异步 runtime 上，不得在 Tauri UI 或 Android 主 Looper 上做阻塞网络调用。

## 各平台落地方式与限制

### 桌面 Tauri

由 Tauri/Rust 创建独立、初始隐藏、禁止导航到前端应用 UI 的 source WebView；关闭时销毁对应浏览器上下文。要实现所有 WebView fetch/XHR 都进 Rust，先选择能覆盖子资源请求的 Tauri/WebView engine hook 或本地受控 origin + 脚本桥；仅靠 Tauri navigation callback 不足以拦截子资源。

主线程调度要异步：JNI 线程提交带 request ID 的执行请求；Rust 通过 Tauri `run_on_main_thread` 排入创建/评估/销毁任务，并用 oneshot 等待结果。等待方必须是 JNI/worker 线程，Tauri 主事件循环继续运行；若调用源本身在主线程，则直接返回明确的不支持/改造错误，不能 `runBlocking` 卡住主线程。托管窗口不可向 Vue 窗口发 source script 或 DOM 事件。

### Android

平台 adapter 建立专用隐藏 `WebView`，其所有创建、导航、注入和销毁均在主 Looper。同步的书源 XHR 若由 `shouldInterceptRequest` 支持，则回调可从 WebView 资源线程桥到 Rust HTTP session，再返回 `WebResourceResponse`；但该 API 不给 POST body，且回调行为/redirect/body 限制需用目标 Android System WebView 实测。更通用的请求体方案需要 document-start XHR/fetch adapter 发往 loopback session proxy。主线程不能 `runBlocking` 等这个请求完成。

### iOS WKWebView

WKWebView 没有等价 Android `shouldInterceptRequest` 的任意 HTTPS 子请求 body hook，`WKURLSchemeHandler` 不接管内建 HTTPS。可测试的候选方案是把隐藏文档放在应用自定义 scheme 下，注册 `WKURLSchemeHandler` 对自定义资源 URL 转发 Rust session，并在 `WKUserScript` document-start 处包装 XHR/fetch，将 `http(s)` 请求改写到 capability scheme。必须先做最小原生 fixture，验证同步 XHR 请求会调用 handler、页面能收到 status/body，DOMParser 解析正常，且没有绕过代理的 HTTPS 请求。若同步 XHR 无法在自定义 scheme 工作，则当前 WKWebView 路线不满足该书源语义，需要另选平台浏览器引擎/明确平台缺口，不可用异步语义或简单 DOM 模拟冒充。

## 分阶段验收

1. 纯逻辑测试覆盖模式区分、请求/结果 wire 格式、session 过期/取消、错误传递，不引入 GTK/WebKitGTK。
2. 每个平台单独的真实浏览器 fixture 覆盖真实 `document.querySelector`、`DOMParser`、同步 XHR、多页 `.con` 合并、重定向与超时；fixture 的 upstream 记录请求，证明每次请求都由 Rust 发出、私有认证信息不会进入 DOM/前端。
3. `sourceRegex` fixture 只在 `SniffResource` 下等待并返回命中 URL；普通 `TransformContentPage` 即使带有同一 regex，也应立即返回处理后的 HTML。
4. 对 `伪速读谷²` 的实际 `webJs` 用本地可控分页站点做端到端：确认隐藏浏览器执行两页/多页 DOM 操作、正文 selector 收到合并后的处理 DOM、Vue 只收到最终正文。确认每个必要平台的原生实现后才将功能标为完成。

在此之前，`ContentRule.webJs` 正文执行和 desktop/Android Backstage factory 注册均为未实现能力。iOS 的现存类也未注册且不能代表已验证支持。
