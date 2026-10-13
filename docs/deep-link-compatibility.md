# 原版深度链接兼容

对照 `D:\sdk\legado` 的 `LegadoDeepLink.kt`、`DeepLinkImportHost.kt`、
`SchemeImportOps.kt` 和四类 `Import*ViewModelShared`。
本次范围是现有功能的协议和导入行为，未新增主题、词典规则和 ZIP 排版功能。

## 协议与分流

桌面和移动端均注册 `legado`、`yuedu`、`legado-rs`。
原版两种协议共用下表，载荷取第一个 `src` 参数。

| 原版路径 | 当前行为 |
| --- | --- |
| `/bookSource`、`/rssSource` | 书源 / RSS 勾选导入 |
| `/replaceRule` | 替换规则勾选导入 |
| `/textTocRule` | TXT 目录规则勾选导入 |
| `/httpTTS` | HTTP TTS 勾选导入 |
| `/importonline`，host 为 `booksource` / `rsssource` | 旧格式书源 / RSS 导入 |
| `/importonline`，host 为 `replace` | 旧格式替换规则导入 |
| `/read` | 书架内按原进度阅读；不在书架则准备书籍并打开详情 |
| `/addToBookshelf` | 准备书籍并打开详情，用户决定是否加入书架；已在书架则提示 |
| 其他路径 | 下载 / 解析 JSON，自动判断四类已有配置 |
| `/theme`、`/dictRule`、`/readConfig` | 明确提示当前没有对应的原版导入功能 |

保留 `legado-rs://import-source?url=...` 和 `legado-rs://import-rules?url=...`。
原版链接缺少或空 `src` 时静默忽略，查询值进行 percent 解码及 `+` 转空格，外层 fragment 忽略。

## 导入流程

冷启动使用插件 `getCurrent`，运行中使用 `onOpenUrl`；桌面复用 single-instance 转发。
收到链接后直接打开导入勾选框。连续请求排队，关闭 / 完成当前请求后处理下一条。

导入载荷支持 HTTP(S) 地址、JSON 对象、JSON 数组。
书源还支持 `{"sourceUrls":["https://example.com/a.json","https://example.com/b.json"]}`。
下载地址的 `#requestWithoutUA` 后缀表示去除该标记且不发送默认 User-Agent。
未知 JSON 按原版字段顺序识别 `bookSourceUrl` / `sourceUrl` / `pattern` /
`themeName` / `showRule` / `name+rule` / `name+url`；未实现的类型会明确报错。

书架导入页的“识别”按钮使用同一入口，支持普通地址、原版深链和直接粘贴 JSON，
不再让用户预先选择书源或替换规则处理器。
Rust 负责下载、规范化和类型识别，预览只在内存中；确认后调用已有书源 / 配置 store 保存，
不创建 `configuration-import-*` 快照。KMP 请求行为保持不变。

书籍链接通过已有书籍准备流程取得详情和目录，匹配明确 `origin`、同站书源或 `bookUrlPattern`。
URL 参数使用 JSON；正则匹配使用本项目 Rust regex 支持的语法。
保留本项目的 HTTP(S)、内容大小和队列数量限制，不读取深链指定的本地文件。

系统协议注册的变更需要重新编译并启动桌面程序，移动端需要重新打包安装。
静态类型与核心库编译检查不代表完成了 Windows / Android / iOS 的系统唤起验证。
