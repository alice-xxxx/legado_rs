export type SourceFieldKind = "text" | "long" | "number" | "toggle" | "type";
export interface SourceEditorField {
  key: string;
  label: string;
  kind: SourceFieldKind;
  defaultValue?: string;
  aliases?: string[];
}
export interface SourceEditorTab { id: string; label: string; fields: SourceEditorField[] }
const field = (key: string, label: string, kind: SourceFieldKind = "long", defaultValue?: string): SourceEditorField =>
  ({ key, label, kind, defaultValue });
const rules = (parent: string, rows: Array<[string, string]>): SourceEditorField[] =>
  rows.map(([key, label]) => field(`${parent}.${key}`, label));
const listRules: Array<[string, string]> = [
  ["bookList", "书籍列表"], ["name", "书名"], ["author", "作者"], ["intro", "简介"],
  ["kind", "分类"], ["lastChapter", "最新章节"], ["updateTime", "更新时间"],
  ["bookUrl", "详情链接"], ["coverUrl", "封面链接"], ["wordCount", "字数"], ["hasMoreRule", "是否还有下一页"],
];
const networkFields: SourceEditorField[] = [
  field("legadoRsUserAgentOverride", "User-Agent"), field("header", "请求头"), field("concurrentRate", "并发率", "text"),
  field("enabledCookieJar", "自动保存 Cookie", "toggle", "true"),
  field("enableDangerousApi", "启用高危 API", "toggle", "false"),
  field("loginUrl", "登录 URL / 登录脚本"), field("loginUi", "登录 UI"), field("loginCheckJs", "登录检查 JS"),
  field("jsLib", "JS 库"), field("coverDecodeJs", "封面解密 JS"), field("variableComment", "变量说明"),
];

// Match BookSource and its six rule models in KMP.
export const bookSourceEditorTabs: SourceEditorTab[] = [
  { id: "basic", label: "基本", fields: [
    { ...field("bookSourceName", "名称", "text"), aliases: ["sourceName", "name"] },
    { ...field("bookSourceUrl", "地址", "text"), aliases: ["sourceUrl", "url"] },
    { ...field("bookSourceGroup", "分组", "text"), aliases: ["sourceGroup", "group"] },
    field("bookSourceType", "类型", "type", "0"),
    { ...field("enabled", "启用", "toggle", "true"), aliases: ["bookSourceEnabled"] },
    field("bookSourceComment", "备注"), field("bookUrlPattern", "书籍 URL 正则"), ...networkFields,
    field("customOrder", "排序", "number", "0"), field("weight", "权重", "number", "0"),
    field("respondTime", "响应时间（毫秒）", "number", "180000"), field("lastUpdateTime", "最后更新时间（时间戳）", "number", "0"),
  ] },
  { id: "search", label: "搜索", fields: [field("searchUrl", "搜索地址"), field("ruleSearch.checkKeyWord", "校验关键词"), ...rules("ruleSearch", listRules)] },
  { id: "explore", label: "发现", fields: [
    field("enabledExplore", "启用发现", "toggle", "true"), field("exploreUrl", "发现地址"),
    field("exploreScreen", "发现筛选规则"), field("exploreStyle", "发现样式", "number", "0"), ...rules("ruleExplore", listRules),
  ] },
  { id: "info", label: "详情", fields: rules("ruleBookInfo", [
    ["init", "初始化规则"], ["name", "书名"], ["author", "作者"], ["intro", "简介"], ["kind", "分类"],
    ["lastChapter", "最新章节"], ["updateTime", "更新时间"], ["coverUrl", "封面链接"], ["tocUrl", "目录链接"],
    ["wordCount", "字数"], ["canReName", "允许重命名"], ["downloadUrls", "下载链接"],
  ]) },
  { id: "toc", label: "目录", fields: rules("ruleToc", [
    ["preUpdateJs", "更新前 JS"], ["chapterList", "章节列表"], ["chapterName", "章节名称"], ["chapterUrl", "章节链接"],
    ["isVolume", "卷标题"], ["isVip", "VIP 章节"], ["isPay", "付费章节"], ["updateTime", "更新时间"], ["nextTocUrl", "下一页目录"],
  ]) },
  { id: "content", label: "正文", fields: [
    ...rules("ruleContent", [
      ["content", "正文内容"], ["title", "章节标题"], ["nextContentUrl", "下一页正文"], ["webJs", "网页 JS"],
      ["sourceRegex", "资源嗅探正则"], ["replaceRegex", "替换规则"], ["imageStyle", "图片样式"],
      ["imageDecode", "图片解密 JS"], ["payAction", "购买操作"],
    ]),
    { ...field("ruleContent.subContent", "附加内容 / 歌词"), aliases: ["ruleContent.lrcRule"] },
    ...rules("ruleContent", [["musicCover", "音乐封面"], ["shouldOverrideUrlLoading", "网页跳转拦截"]]),
  ] },
  { id: "review", label: "段评", fields: [field("enabledReview", "启用段评", "toggle", "true"), ...rules("ruleReview", [
    ["reviewUrl", "评论列表地址"], ["reviewList", "评论列表"], ["reviewCountRule", "各段评论数"], ["reviewIdRule", "评论 ID"],
    ["avatarRule", "头像"], ["nameRule", "用户名"], ["contentRule", "评论内容"], ["postTimeRule", "发布时间"],
    ["extraRule", "附加信息"], ["imagesRule", "图片列表"], ["voteUpCountRule", "点赞数"], ["voteUpSelectedRule", "是否已点赞"],
    ["voteDownSelectedRule", "是否已点踩"], ["replyCountRule", "回复数"], ["totalCountRule", "评论总数"],
    ["replyListUrl", "回复列表地址"], ["hasMoreRule", "是否还有下一页"], ["voteUpRule", "点赞操作"],
    ["voteDownRule", "点踩操作"], ["replyRule", "回复操作"], ["deleteRule", "删除操作"],
  ])] },
];

// Legacy RSS sources use flat rules instead of BookSource's nested rules.
export const rssSourceEditorTabs: SourceEditorTab[] = [
  { id: "basic", label: "基本", fields: [
    field("sourceName", "名称", "text"), field("sourceUrl", "地址", "text"), field("sourceGroup", "分组", "text"),
    field("enabled", "启用", "toggle", "true"), field("sourceComment", "备注"), field("sourceIcon", "图标"), ...networkFields,
    field("customOrder", "排序", "number", "0"), field("lastUpdateTime", "最后更新时间（时间戳）", "number", "0"),
  ] },
  { id: "explore", label: "发现", fields: [
    field("sortUrl", "分类地址"), field("singleUrl", "单一地址", "toggle", "false"), field("articleStyle", "列表样式", "number", "0"),
    field("ruleArticles", "文章列表"), field("ruleNextPage", "下一页"), field("ruleTitle", "标题"), field("rulePubDate", "发布时间"),
    field("ruleDescription", "简介"), field("ruleImage", "图片"), field("ruleLink", "文章链接"),
  ] },
  { id: "content", label: "正文", fields: [
    field("ruleContent", "正文内容"), field("contentWhitelist", "内容白名单"), field("contentBlacklist", "内容黑名单"),
    field("shouldOverrideUrlLoading", "网页跳转拦截"), field("style", "页面 CSS"), field("enableJs", "启用页面 JS", "toggle", "true"),
    field("loadWithBaseUrl", "使用基础地址加载", "toggle", "true"), field("injectJs", "注入 JS"),
  ] },
];
