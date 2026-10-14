# 应用数据与共享状态

以下描述当前代码的数据位置；字段以 Rust 模型和 `src/api/types.ts` 为准。

## 持久化

| 文件或目录 | 内容 |
|---|---|
| `settings.json` | 应用、阅读器、主页和 WebDAV 配置。 |
| `content-library.json` | 内容库预览、顺序和分组。 |
| `history.json` | 阅读会话、统计、搜索历史和书签。 |
| `sources.json` | 完整书源定义。 |
| `replacement-rules.json` | 正文显示替换规则。 |
| `txt-toc-rules.json` | 本地 TXT 目录识别规则。 |
| `http-tts.json` | HTTP TTS 配置。 |
| `tasks.json` | 持久化任务记录。 |
| `rss-state.json` | RSS 状态。 |
| `cache/{类型}/{id}/` | 完整内容 JSON、正文、资源和导入原文件。 |
| `source-engine/` | 引擎数据、登录状态和发现分类映射。 |
| `transactions/` | 多文件写入的恢复日志。 |

书籍的引擎执行上下文和媒体请求映射保存在同一份内容 JSON 的私有字段中。HTTP 响应由 Rust 投影并过滤私有字段。搜索结果留在内存，不作为文件保存。

内容库与缓存格式见[缓存说明](content-library-cache-design.md)。加入或移出内容库不改变缓存 ID；移出内容库保留缓存，不在启动时按内容库成员删除缓存。

## Vue 共享状态

| 状态入口 | 用途 |
|---|---|
| `settingsStore.ts` | 全局设置。 |
| `historyStore.ts` | 全局历史和书签。 |
| `configurationStores.ts` | 替换规则、TXT 目录规则、HTTP TTS。 |
| `contentLibraryState.ts` | 内容库预览和分组；`shelfStore.ts`、`shelfBooks.ts` 提供派生视图。 |
| `sourcesStore.ts` | 轻量书源列表和单条定义的读写入口。 |
| `bookDetailState.ts` | 详情与阅读共用的 `openedBook`，以及轻量 `bookPreviews`。 |
| `tasksStore.ts` | 任务列表和更新。 |

这些入口位于 `src/features/` 下相应功能目录，由 `src/main.ts` 初始化。书籍完整资料按需加载；按 ID 打开时先调用 `get_book` 取得描述符，再通过统一入口读取资源。

Vue 修改共享状态或提交编辑字段，Rust 持久化并通知回读。编辑书籍只提交显示覆盖字段，不提交完整章节或进度。新建和编辑书源共用编辑器，完整规则只保存在编辑草稿中。

内容库排序保存在 WebView 的 localStorage，分组和成员写入 `content-library.json`。换源时，已有内容库成员替换为目标缓存的预览；非成员只切换当前内容。
