# 本地书籍导入

Rust 解析并持久化，Vue 读取缓存和展示。四种格式均在完成后加入内容库。

| 格式 | 缓存目录 | 内容 |
|---|---|---|
| TXT | `cache/text/{id}/` | `{id}.json`、`original.txt`、章节 TXT。 |
| EPUB | `cache/html/{id}/` | `{id}.json`、`original.epub`、章节 HTML、封面和本地资源。 |
| CBZ | `cache/image/{id}/` | `{id}.json`、`original.cbz`、`c001_p001.webp` 等图片页。 |
| PDF | `cache/pdf/{id}/` | `{id}.json`、`original.pdf`。 |

流程：选择文件 → 临时副本 → 按内容、格式和解析选项确定 `local-{hash}` ID → 保存原文件 → 解析、写入资源和索引 → 加入 `content-library.json`。

重复导入相同内容与选项会复用缓存，保留进度、显示覆盖和分组；文件原路径和名称不参与 ID。解析失败时清理未完成缓存；上架失败时保留已生成的缓存并报错。

- **TXT**：使用启用的 TXT 目录规则；无配置时使用内置识别，全部禁用或没有匹配时按正文分段。标题不重复写入正文。支持 UTF-8、带 BOM 的 UTF-16、GBK，以及显式字符集选项。无目录约每 10 Ki 字符分段，有目录长章约每 100 Ki 字符分段。
- **EPUB**：读取 EPUB 3 导航或 EPUB 2 NCX，按 spine 和片段边界分章；不可用的目录回退到 spine。提取书名、作者、简介和封面，重写本地资源链接。阅读器移除书内样式，使用阅读设置和默认段落缩进，不保证还原 EPUB 的全部原始排版。
- **CBZ**：一个文件作为一个阅读单元，图片自然排序。已有 WebP 校验后复用，其他格式转为无损 WebP；存在 ComicInfo.xml 时读取系列、章节、作者和简介。
- **PDF**：保存原文件及页数，运行时生成页索引，不创建逐页正文缓存。加密文件通过临时导入令牌重试密码，密码不写入 JSON。

代码入口：`src-tauri/src/books/local_books.rs`、`src-tauri/src/application/file_import.rs`、`src/features/books/useLocalBookImport.ts`。

回归检查，在仓库根目录运行：

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib --no-default-features local_books::import_tests
```
