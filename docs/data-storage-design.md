# 应用数据结构与前端共享状态约定

本文记录已经确认的数据文件边界和 Vue 状态约定。它描述目标结构；没有列出的字段继续以项目现有 Rust/TypeScript 类型为准，不在这里另造兼容字段或迁移逻辑。

## 数据目录

~~~text
app-data/
├─ settings.json
├─ shelf.json
├─ history.json
├─ replacement-rules.json
├─ txt-toc-rules.json
├─ http-tts.json
├─ sources.json
├─ tasks.json
├─ rss-state.json
├─ books/
│  └─ {缓存书ID}/
│     ├─ book.json
│     ├─ source-state.json
│     ├─ original.{txt,epub,cbz,pdf}
│     ├─ chapters/
│     ├─ assets/
│     └─ media-maps/
├─ source-engine/
│  ├─ discovery-categories/
│  └─ …（现有引擎数据）
├─ transactions/
└─ search/
~~~

source-engine/ 保留现有引擎数据的位置和存储逻辑，并保存书源级分类 URL 映射。它是 Rust 私有数据；其他面向 WebView 的 JSON 和资源由前端按资源描述自行读取，不为读取这些数据另加 Rust IPC。

## 文件职责

| 文件或目录 | 保存内容 |
|---|---|
| settings.json | 应用级设置、阅读器设置、主页配置及 webdavBackup 下的 WebDAV 连接和备份调度配置。主页配置保存在这里，不单独建文件；所有设置修改都写回这一份完整 JSON。 |
| shelf.json | 书架顺序和位置，只保存书籍 ID 等位置信息，不重复保存书名、作者等详细资料。 |
| history.json | 阅读记录、阅读统计、搜索历史和书签。按 reading、search、bookmarks 三部分组织。 |
| replacement-rules.json | 阅读正文使用的文字替换规则。 |
| txt-toc-rules.json | 导入本地 TXT 书籍时识别章节标题的规则。 |
| http-tts.json | HTTP 语音朗读服务配置。 |
| books/{缓存书ID}/book.json | 缓存书的详细信息、阅读进度，以及可切换来源对应的缓存书 ID。是否在书架由 shelf.json 决定，书签不放在这里。 |
| books/{缓存书ID}/source-state.json | Rust 后续解析使用的原始书籍和章节数据，包括 URL、规则变量及目录版本标记；不向 WebView 发布。 |
| books/{缓存书ID}/original.* | 导入书籍的永久原文件，不属于导入暂存。 |
| books/{缓存书ID}/media-maps/ | 本书音视频资源 ID 对应的真实 URL 和请求头；仅由 Rust 读取，不向 WebView 发布。 |
| source-engine/discovery-categories/ | 按书源 ID 保存分类 ID 与发现 URL 的映射；仅供 Rust 执行发现请求。 |
| sources.json | 根目录的权威书源定义；位置不变。 |
| transactions/ | 多文件写入事务的恢复日志，不包含在备份中。 |
| books/{缓存书ID}/chapters/ | 这本缓存书的章节数据。 |
| books/{缓存书ID}/assets/ | 这本缓存书使用的本地资源，例如封面。 |
| search/ | 临时搜索结果快照，提供给 WebView 按资源描述读取，不作为搜索历史。 |

书源修改计数只保存在 Rust 当前进程的内存中，用于拒绝搜索、换源、章节加载和网页登录的旧异步结果，不单独持久化或备份。修改、删除或重新导入书源会推进对应计数；恢复备份后推进已记录的所有计数。它与前端轻量列表的更新通知无关。

settings.json 的字段沿用现有设置类型：应用级设置在根对象中，阅读器设置放在 reader，主页配置放在 home。这里不额外造 app 子对象或重复字段。
WebDAV 完整配置保存在 settings.json.webdavBackup 中，与其他设置一起读取到唯一的全局 settings。连接表单只复制地址、用户名和输入密码的草稿，保存后回写 settings；自动备份间隔直接修改 settings。两者都由 settings 的 watch 提交完整 JSON，不另设保存命令。密码输入框不回填；备份包含这份 settings，恢复时沿用现有的本机 WebDAV 连接保留行为。

文件选择和解析过程的中间副本使用系统临时目录，完成或取消后清理；导入成功的原文件永久保存在 books/{缓存书ID}/original.*，不会因移出书架被清理。

history.json 中，reading 保存阅读会话、按书和按日期的统计及总计；search 保存搜索历史。书签保存在 bookmarks 字段中，按书籍 ID 组织完整记录，每条包含章节 ID、章节位置、备注等书签内容；不另建书签文件。


### 缓存书与书架引用

- books/ 中一律是缓存书，不区分书架书与非书架书，也不另设独立的书架书 ID。
- shelf.json 决定哪些缓存书放在书架及其顺序；书籍详细信息从对应的 books/{缓存书ID}/book.json 读取。
- 从书架进入的书籍换源时，使用新来源的缓存书 ID，在 shelf.json 中原位置替换旧 ID，并让 openedBook 加载新 ID 对应的 book.json。
- 从其他界面换源时，只切换 openedBook 使用的缓存书，不修改 shelf.json，也不因换源自动加入书架。
- 不能因为缓存书不在 shelf.json 中就删除它；启动时不按书架成员清理缓存。换源后旧缓存也继续保留。

### 不单独保留的项目

阅读进度归入对应缓存书的 book.json；搜索历史和书签归入 history.json。因此不再另设 reading/、progress/、private-data/、discovery/、bookmarks.json 或 reading-history.json。search/ 仍保留，但只存临时搜索结果快照。

## 前端与 Rust 的读写边界

- Rust 是持久化数据和业务状态的写入方；Vue 维护响应式界面状态。
- Rust 向前端发布资源描述或更新事件；前端通过统一的资源读取入口读取对应 JSON，不通过 IPC 返回这些文件的 JSON 内容。WebDAV 读取来自共享 settings；阅读统计由 Vue 从共享 history 的会话记录计算，不再单独调用读取 IPC。
- Vue 向 Rust 保存时只提交该界面允许修改的字段。书籍显示信息保存只包含书籍 ID 和书名、作者、简介、封面等可编辑字段，不携带章节、阅读进度或缓存数据(只对book.json有效)。
- 删除旧字段兼容、自动清理旧配置等迁移逻辑；项目不需要兼容旧格式。

## 全局响应式状态

### settings

- 在 settings 专属的 TypeScript store 中只维护一个全局响应式 settings。
- 应用启动时只加载一次；其他页面直接使用同一份状态，不自行调用读取接口。
- 页面直接修改响应式状态。store 监听变化，把完整 settings JSON 和递增的请求 ID 发给 Rust。
- Rust 更新事件带回请求 ID 和新资源描述。前端只应用最新请求对应的结果；回读替换状态时暂停 watch，替换完成后再恢复，避免把 Rust 的回读再次保存。
- 不额外维护 settingsReady、settingsSaving、settingsDirty、settingsSaveError、等待队列或多层 revision 状态；初始化由 main.ts 负责，保存由 watch 负责。

### history

- history.json 也由一个专属 TypeScript store 提供全局响应式 history；页面共用同一份状态。
- 修改由 store 监听并统一保存；Rust 更新时由同一套通知和回读流程更新 Vue 状态。

### shelf
- shelf.json 也由一个专属 TypeScript store 提供全局响应式 shelf；页面共用同一份状态。
- 修改由 store 监听并统一保存；Rust 更新时由同一套通知和回读流程更新 Vue 状态。

### sources

- sourcesStore.ts 集中管理书源列表读取、更新通知、单条定义读取、保存、导入和导出。main.ts 在启动时只加载一次轻量列表，页面共享 sources，不自行读取或维护另一份列表。
- sources 只包含 ID、名称、分组、启用状态、类型和能力等列表字段，不包含完整规则或脚本，也不安装自动保存的深度 watch。
- Rust 比较保存前后的列表字段，只有字段变化或列表增删时才发送 sources-updated。sourcesStore.ts 收到通知后回读轻量列表；只修改规则正文不刷新列表。备份恢复通过 app-state-updated 回读列表。
- source-list.json 是 HTTP 按需生成的轻量资源地址，不对应额外的磁盘文件。单条 sources/{书源ID}.json 同样按请求从权威 sources.json 生成，完整定义用 definitionJson 字符串传输，保留 64 位数字。
- 编辑器按 ID 向 sourcesStore.ts 请求一条定义，保存在页面局部草稿中；保存也交给同一入口提交 Rust。关闭或切换编辑器后释放草稿，不建立全局 openedSource 或完整规则缓存。保存方法在有列表变化时等待对应更新通知完成，页面无需再次刷新列表。
- 新建和编辑共用分类字段表单，嵌套规则展开成具体字段，不提供完整 JSON 编辑入口。缺失字段仍显示空值或默认值；仅修改过的字段在点击保存时写入定义，按需补齐规则对象。切换分类不提交或丢弃草稿，长脚本在独立编辑区域修改。
- Rust 的权威 sources.json 仍保存全部原始定义；单条读取减少前端的数据传输和解析，Rust 端仍需读取这个完整文件。

### openedBook

- 详情页和阅读页共用一个全局响应式 openedBook。这两类页面互斥；无关页面不需要另存一份书籍状态。
- 所有 book.json 的读取集中在 openedBook 所在的 bookDetailState.ts；其他模块只能请求这个入口加载，并使用其全局响应式状态，不能自行读取书籍 JSON。
- 同一文件提供全局响应式 bookPreviews，按缓存书 ID 保存轻量预览。书架、历史、书签及已缓存换源候选共用这份预览；缺少某个 ID 时，通过统一入口读取对应 book.json，只复制所需字段，不保留完整书籍或章节数组。
- 预览只包含书名、作者、封面、类型、章节数、最新章节、少量来源信息及独立复制的进度字段。预览没有保存 watch，也不能作为书籍保存载荷；书籍编辑仍通过 openedBook。
- Rust 更新书籍 JSON 后，同一入口回读并更新当前 openedBook；若 bookPreviews 已有该缓存 ID，就更新其预览，没有则不新增。非当前书籍的更新只刷新已有预览，不替换 openedBook。
- 预览按需加载，同一 ID 的并发请求共用一次读取，批量请求逐本读取并释放完整目录。关闭或切换书籍后，不在另一个全局集合中保留其完整 book.json。
- 切换书籍时通过统一入口加载 book.json。阅读器从 openedBook 获取当前书籍和缓存资源信息；请求 Rust 缓存章节后，Rust 通过同一个 book-updated 通知更新 book.json。
- book-updated 事件监听保持安装，不随详情页开关而移除。收到事件后由统一函数读取资源、更新 openedBook；替换 Vue 状态时只暂停状态 watch，避免把 Rust 回读当成用户修改再发回。
- 阅读页不另存一份书籍对象，直接从 openedBook 取当前书籍及缓存地址。章节内容请求通过阅读器的章节缓存请求函数发给 Rust，阅读进度通过对应的阅读进度操作写入；两类操作如果更新 book.json，都走同一个 book-updated 通知和 openedBook 回读函数。书籍显示信息的 watch 只提交前端可编辑的显示字段。

### 书籍信息编辑

- 打开编辑界面时，只把 openedBook 中要编辑的字段复制到界面本地草稿：书名、作者、简介和封面。
- 编辑期间只改本地草稿；取消时不影响 openedBook。
- 保存时把这些字段写回全局 openedBook，由其 watch 自动向 Rust 保存；不复制整本书，也不把章节、进度或缓存字段交给编辑表单。
- 保存由 watch 直接发起，不保存“最近一次保存 Promise”，也不为了等待它额外维护保存中状态。

### 强调点

- settings.json、shelf.json、history.json 在启动时各加载一次；所有界面共用对应的全局响应式状态。UI 直接读取这份 JSON；操作只修改它，由 store 的 watch 保存和通知回读。不要另存展示副本、可写 computed 转换层、手动刷新或保存等待状态。书籍资料按缓存 ID 从对应 book.json 读取，书架展示组合与排序是只读派生数据，不反向写回 shelf。
- 需要保存按钮的表单只复制所需字段到本地草稿，点击保存后写回全局状态；自动保存的操作直接修改全局状态。两者都由 store 的 watch 向 Rust 保存。
- 书架根据 shelf 中的缓存书 ID 读取对应 book.json 来显示书籍；books/ 里的每本书都是缓存书，加入书架只是在 shelf 中引用已有缓存 ID。
- 详情页和阅读页共用 openedBook。书籍信息编辑只回写 Vue 可以编辑的显示字段，不能向 Rust 提交完整 book.json。
- replacement-rules.json、txt-toc-rules.json、http-tts.json 各自只有一份全局完整响应式 JSON，在启动时加载一次，列表、编辑器和阅读器共用。单项编辑器只复制当前规则可编辑字段到本地草稿，保存回写全局状态，自动操作直接修改全局状态。
- 这三类配置的 watch 只向 Rust 提交变动的一条完整规则，删除提交该规则 ID 和空值，不提交整个文件。Rust 合并保存后通知规则 ID、请求 ID 和资源描述；Vue 自行回读并暂停 watch 更新对应规则，不能用一条规则的结果覆盖其他规则的本地修改。
- 导入命令只选文件或下载、解析和发布导入资源，不直接修改配置文件。Vue 读取导入资源并合并到全局 JSON，由同一 watch 逐条保存。
- 搜索和发现页点击书籍进入详情；书架点击进入阅读，长按进入详情。加入书架必须在详情页操作。
- 搜索或发现中尚未缓存的书，点击后先显示加载界面，向 Rust 请求缓存；取得缓存 ID 后，通过统一入口让 openedBook 加载对应 book.json。
- 搜索结果进入详情时，将选中的书及可换来源一起缓存，并将来源缓存 ID 写入 book.json 的 sourceBookIds；发现结果没有可换来源时，该列表为空。
- 详情页刷新换源时，Rust 搜索书名和作者匹配的书并缓存，将对应缓存 ID 写入当前 book.json，再通过 book-updated 通知前端回读。
- 选择新来源时，让 openedBook 加载新缓存 ID。只有从书架进入的换源操作才在 shelf 原位置替换旧 ID；其他界面的换源操作不修改 shelf。
- 缓存是否保留不取决于它是否在书架，不自动删除未上架或换源后留下的缓存。

## 发现页与刷新交互

### 页面内容和空状态

- 启动后进入发现页。
- 发现内容来自用户配置的栏目、书源和分类，不额外生成或插入推荐内容。空状态判断基于是否配置栏目和当前栏目是否有可显示内容。
- 页面从无内容进入时自动刷新一次；已有内容时进入不自动刷新。手动刷新保留。
- 删除“重新加载”这条额外逻辑，不因从其他页面返回而重新发起加载，也不重复显示“正在从源加载发现内容…”。
- 删除固定只刷新前四个栏目的上限。
- 空状态不套有色边框或背景框，不保留多余的指南文字、装饰图标和重复的栏目管理入口。没有加载内容时也不能留下看起来像加载卡片的空框。
- 封面流具体由哪种数据或接口提供，尚未在对话中确定，不能在文档中臆造数据来源。

### 下拉刷新

- 桌面端和移动端使用同一种下拉刷新交互，不能要求必须先有内容；空状态也能触发。
- 手势下拉时显示刷新提示和下拉距离；达到阈值后提示“松开刷新”，只有松手才执行刷新。移动端以触摸结束作为松手；桌面端通过顶部的滚轮或触控板下拉手势显示同一提示，并在手势结束且达到阈值后刷新。
- 刷新提示应出现在“全部”等分类行下方，不应固定在窗口最顶部。
- 手动刷新入口继续保留，进入页面时的空内容自动刷新不能替代手动刷新。

## 全局安全区与空间分配

- 多个移动端页面顶部曾贴到屏幕最上缘，顶部安全区域应由全局 style.css 中共用的 safe-area-top 类处理，各页面使用同一类。
- 不给每个页面单独设置固定的安全区高度或类似 --safe-area-header-height: 56px 的补丁。页头和内容由各自布局自然分配可用空间。
- 采用全局类的目的，是统一处理并减少重复样式；不要再通过每个页面的专属类把同一规则写一遍。

## Vue 代码的可读性和精简要求
- 可以合并重复代码、采用直接写法并删除没有实际作用的判断和中转层。不要为了“健壮”额外增加不必要的状态、等待 Promise、队列、兼容分支或多套事件入口。
- 全局 settings/history/shelf/book、三类规则状态和轻量书源列表按本文约定集中管理。不要让各页面重复读取，也不要为同一 JSON 维护彼此分离的 Vue 更新路径。
- requestId 只用于区分 Rust 返回的保存结果是否仍对应最新一次修改；不再叠加 revision、waiter、latestSave 等等待系统。
- 编辑书籍信息的草稿只复制表单涉及的字段，不复制整本 openedBook；保存时也只回写这些字段。
- backgroundBusy 是外观设置页面的局部状态：选择或清除阅读背景图片期间禁用相关操作，避免同一异步操作重复启动。它不是 settings 全局保存状态。类似状态只有在确实保护异步操作时才保留。

## JSON 内容示例

以下示例说明每个文件放什么，不是要求把示例值照抄到用户数据中。示例省略运行时资源 URL、实际书目和实际规则内容。

### settings.json

应用级设置放根对象；reader 只放阅读和播放设置；home 放主页标签及栏目配置。

~~~json
{
  "schemaVersion": 1,
  "sourceHttpTimeoutSeconds": 15,
  "reader": {
    "fontSizePx": 19,
    "lineHeight": 1.8,
    "fontFamily": "serif",
    "textColor": "#3f3b34",
    "backgroundColor": "#f7f3e9",
    "backgroundImageOpacity": 20,
    "textAlign": "justify",
    "theme": "paper",
    "preloadCount": 5,
    "verticalScroll": false,
    "audioPlaybackRate": 1,
    "audioSkipSeconds": 15,
    "videoPlaybackRate": 1,
    "ttsEngine": "system",
    "ttsRate": 1,
    "ttsVoiceURI": "",
    "httpTtsConfigId": null,
    "ttsContinueAcrossChapters": true,
    "ttsStartPosition": "current"
  },
  "home": {
    "tabs": [
      {
        "id": "tab-home",
        "title": "主页",
        "sortOrder": 0,
        "sections": []
      }
    ]
  }
}
~~~

### shelf.json

书架只按顺序保存书籍 ID，不重复存书名、作者、封面、进度等书籍详情。详情从 books/{id}/book.json 读取。

~~~json
{
  "books": [
    { "id": "book-668dc1ecb9aaf944" },
    { "id": "book-another-id" }
  ]
}
~~~

### history.json

reading 放阅读会话和统计；search 放搜索历史；bookmarks 按书籍 ID 保存包含章节 ID 和书签内容的记录。

~~~json
{
  "schemaVersion": 1,
  "reading": {
    "sessions": [],
    "books": [],
    "days": [],
    "totalDurationMs": 0,
    "totalSessions": 0
  },
  "search": {
    "entries": [],
    "processedSearchIds": []
  },
  "bookmarks": {
    "book-668dc1ecb9aaf944": [{"id": "bookmark-001", "chapterId": "chapter-001", "chapterIndex": 0, "offset": 0, "note": "", "createdAtMs": 0, "updatedAtMs": 0}]
  }
}
~~~

### replacement-rules.json

保存正文显示时使用的替换规则，不放进 settings.json。

~~~json
{
  "schemaVersion": 1,
  "rules": [
    {
      "id": "rule-001",
      "name": "示例规则",
      "pattern": "待替换文字",
      "replacement": "替换后的文字",
      "enabled": true,
      "isRegex": false,
      "scope": "all"
    }
  ]
}
~~~

### http-tts.json

保存 HTTP 朗读服务列表和每个服务自己的配置；settings.json 的 reader.httpTtsConfigId 只记录当前选中的服务 ID。

~~~json
{
  "schemaVersion": 1,
  "configs": [
    {
      "id": "tts-001",
      "name": "示例服务",
      "enabled": true,
      "config": {}
    }
  ]
}
~~~

### txt-toc-rules.json

保存本地 TXT 书籍的目录识别正则规则。每项包含稳定 ID、名称、正则表达式、可选示例、排序值和启用状态；这类规则由 Rust 用于解析本地 TXT，不属于书源规则，也不放进 source-engine/。

~~~json
{
  "schemaVersion": 1,
  "rules": [
    {
      "id": "txt-toc-001",
      "name": "常见章节标题",
      "rule": "^第.+章.*$",
      "example": "第1章 山水之间",
      "serialNumber": 0,
      "enable": true
    }
  ]
}
~~~

### books/{缓存书ID}/book.json

保存这本书的详情、sourceId、可切换来源缓存 ID、阅读进度和章节数据。这里不放书架顺序，也不放书签。

~~~json
{
  "schemaVersion": 1,
  "id": "book-668dc1ecb9aaf944",
  "title": "书名",
  "author": "作者",
  "intro": "简介",
  "sourceId": "source-001",
  "sourceBookIds": ["book-001"],
  "chapterCount": 0,
  "progress": {
    "chapterId": null,
    "chapterIndex": 0,
    "offset": 0,
    "updatedAtMs": 0
  },
  "chapters": []
}
~~~

缓存书自己的 books/{缓存书ID}/book.json 同样使用 sourceId 标识来源。缓存书目录 ID 是缓存书 ID；不能根据目录名或书名反推 sourceId。


## 数据约定与现有实现的核对

- 本文的目标结构以用户确认的约定为准；若 Rust 或 TypeScript 类型与这里描述的目标冲突，需要后续单独修改实现，不要悄悄把实际代码结构误写成用户已经确认的设计。
- 用户确认的 shelf.json 目标是只保存书籍 ID 和位置顺序；当前 TypeScript ShelfResource 若仍包含书名、封面、进度等重复信息，应视为待对齐项。
- 缓存书 ID 不因加入书架而改变；当前实现若在加入书架时另建 ID，或换源时在旧 ID 下覆盖来源和目录，应视为待对齐项。书架入口换源应替换 shelf.json 中的 ID，其他入口换源不修改 shelf.json。
- 用户要求不做旧配置兼容；
