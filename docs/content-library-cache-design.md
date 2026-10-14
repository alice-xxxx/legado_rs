# 内容库与缓存

## 目录

```text
app-data/
├─ content-library.json
└─ cache/
   └─ {text|html|image|audio|video|pdf}/
      └─ {id}/
         ├─ {id}.json
         ├─ original.*       # 本地导入时保留
         └─ 正文、封面和媒体资源
```

内容 ID 标识一份缓存，不按书名共用目录。目录内资源平铺；TXT/HTML 正文文件名由章节与内容哈希生成。应用的其他数据见[数据说明](data-storage-design.md)。

## 内容库与完整资料

`content-library.json` 按六种类型保存预览数组，分组使用 `{name, id: [内容ID]}`。预览包含 `id`、`title` 和已有的作者、封面、目录总数、最新标题及阅读摘要；`readAt`、`updatedAt` 使用秒。

`{id}.json` 保存完整资料、目录、进度及必要的引擎状态。内容库成员由预览列表决定，不由缓存目录决定。

- 加入内容库时提取预览；更新完整资料时只刷新已存在的预览。
- 移出内容库会移除预览和分组引用，保留缓存。
- 普通详情与阅读入口不自动上架；[本地导入](local-book-import.md)完成后自动上架。
- 分组的删除不会删除内容缓存。

## 磁盘格式与阅读视图

| 类型 | 磁盘中的正文或资源索引 | 阅读方式 |
|---|---|---|
| text | 章节 `url` 引用同目录 `.txt`。 | 纯文本。 |
| html | 章节 `url` 引用 `.html`，本地图片等放在同目录。 | 净化后的 HTML 片段；EPUB 段落默认首行缩进两字。 |
| image | 章节 `pages` 数组保存页 ID 和 `url`。 | 有序图片页。 |
| audio / video | 章节入口、可选 `sources` 和本地依赖索引。 | 媒体播放；HLS 使用播放列表及其依赖。 |
| pdf | 文档 `url` 与 `pageCount`，导入时保留 `original.pdf`。 | PDF.js 按页读取原文档。 |

原始规则参数存于 `engine`、`engineData`、`engineState` 等私有字段，媒体请求映射存于 `media`。Rust 生成公开响应时过滤这些执行字段，并补充章节序号、资源描述符、媒体信息或 PDF 页视图。

同目录资源在磁盘 JSON 中使用文件名，也可保存稳定的 `resource://` 引用或远程地址。HTTP 响应为这些资源生成运行时 `src`；端口和访问 token 不持久化。可播放的远程或代理地址不表示已经离线缓存。

## 阅读进度

磁盘进度包含 `updatedAtMs`，位置按类型记录：

| 类型 | 位置 |
|---|---|
| text / html | `chapterId`、`charOffset`；HTML 可带段落 `anchor`。 |
| image | `chapterId`、`pageId`、`offsetRatio`。 |
| audio / video | `chapterId`、`timeMs`；视频可带 `quality`。 |
| pdf | 从 1 开始的 `page`。 |

前端读取的是投影后的统一进度，不应直接用磁盘格式替代 `BookResource`。格式转换以 `src-tauri/src/resources/content.rs` 为准。

## 搜索

`start_search` 指定 `text`、`audio`、`image` 或 `video`，没有 HTML/PDF 搜索类型。结果通过 Tauri Channel 增量发送，`get_search_task_results` 读取当前内存结果；不生成搜索结果文件。

`resultId` 是候选标识，不是内容 ID。打开候选后 Rust 准备选中来源的完整资料，将其他来源保存在 `alternatives`；实际换源时才准备目标来源的缓存。任务释放不删除已生成的缓存。
