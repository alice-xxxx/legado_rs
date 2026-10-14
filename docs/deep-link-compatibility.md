# 深度链接

Tauri 配置注册 `legado`、`yuedu`、`legado-rs`。解析入口为 `src/features/import/deepLinks.ts`，处理入口为 `useExternalImportActions.ts`。

## 路径

`legado`、`yuedu` 使用第一个 `src` 查询参数，缺失或为空时忽略。

| 路径 | 行为 |
|---|---|
| `/bookSource`、`/rssSource` | 书源或 RSS 配置勾选导入。 |
| `/replaceRule`、`/textTocRule`、`/httpTTS` | 替换规则、TXT 目录规则或 HTTP TTS 勾选导入。 |
| `/importonline` | 按 host 区分旧式书源、RSS、替换规则，其他情况自动识别。 |
| `/read` | 已在内容库则继续阅读，否则准备书籍并打开详情。 |
| `/addToBookshelf` | 准备详情，由用户决定加入内容库；已在库中则提示。 |
| `/theme`、`/dictRule`、`/readConfig` | 提示未提供对应导入功能。 |
| 其他路径 | 自动识别已有配置类型。 |

`legado-rs://import-source?url=...` 和 `legado-rs://import-rules?url=...` 分别导入书源和替换规则。

## 导入

冷启动和运行中的链接共用队列与导入弹窗；内容库导入页的粘贴入口也使用同一解析流程。载荷可为 HTTP(S) 地址或 JSON，Rust 下载、识别和校验，确认后由对应 store 保存。书籍链接通过已有书籍准备流程解析，不读取链接指定的本地文件。

协议注册见 `src-tauri/tauri.conf.json`。配置注册与系统实际唤起是不同检查，修改注册后需要重新构建、安装。
