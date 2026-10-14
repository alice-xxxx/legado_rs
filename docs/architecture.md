# 架构

## 职责

| 层 | 职责 |
|---|---|
| Vue | 页面交互、共享响应式状态、读取资源、排版与播放。 |
| Rust | 应用服务、持久化、任务、缓存、资源服务与引擎宿主能力。 |
| KMP | 执行书源规则和脚本。桌面通过 JVM/JNI，Android/iOS 通过原生插件接入。 |

应用命令由 Tauri IPC 调用。书籍和配置以资源描述符交付，Vue 通过 HTTP 读取；搜索使用 Tauri Channel 发送结构化结果。运行时资源地址不写入长期数据。

## 代码入口

| 路径 | 用途 |
|---|---|
| `src/main.ts`、`src/app/RoutesRoot.vue`、`src/router/` | 初始化共享状态、挂载页面及独立功能 host、哈希路由。 |
| `src/features/` | 页面、阅读器、导入、配置及对应状态。 |
| `src/api/`、`src/ui/` | 类型化命令与资源接口、共用展示组件。 |
| `src-tauri/src/application/` | ApplicationService 和 Tauri 命令。 |
| `src-tauri/src/books/`、`sources/`、`discovery/`、`rss/` | 本地书籍、书源引擎、发现与 RSS。 |
| `src-tauri/src/resources/`、`backup/`、`models/` | 文件及 HTTP 资源、事务恢复、备份、数据类型。 |
| `src-tauri/src/reading_tools/`、`search/`、`settings/` | 阅读记录、规则、搜索历史与朗读配置。 |
| `kotlin/`、`src-tauri/plugins/source-engine/` | KMP 引擎、原生库及移动端插件。 |

## 数据与阅读

内容库成员和内容缓存分别保存。完整资料位于 `cache/{类型}/{id}/{id}.json`；正文为 TXT/HTML 文件，漫画使用图片页，PDF.js 读取原 PDF。具体格式见[缓存说明](content-library-cache-design.md)，共享状态见[数据说明](data-storage-design.md)。

阅读位置通过 `notify_reading` 保存，准备的章节通过 `book-updated` 发布。正文文件名包含章节与内容哈希；设置和完整内容 JSON 则在原路径更新。显示替换和阅读样式不回写正文缓存。

搜索类型为文本、音频、图片、视频。结果与候选保存在 Rust 当前会话内存中，按任务身份发布增量结果及排序；选择候选后才准备内容缓存。书源规则不在展示前端执行。
