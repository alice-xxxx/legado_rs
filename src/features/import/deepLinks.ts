import type { ConfigurationImportKind } from "../../api/settings";
export type ImportKind = ConfigurationImportKind;
export type DeepLinkKind = ImportKind | "auto" | "read" | "addToBookshelf" | "theme" | "dictRule" | "readConfig";
export type PendingImportLink = { kind: DeepLinkKind; url: string };

export const deepLinkLabels: Record<DeepLinkKind, string> = {
  sources: "书源 / RSS", replacementRules: "替换规则", txtTocRules: "TXT 目录规则",
  httpTtsConfigs: "HTTP TTS", auto: "配置", read: "阅读书籍", addToBookshelf: "书籍详情",
  theme: "主题", dictRule: "词典规则", readConfig: "ZIP 排版配置",
};

/** Match LegadoDeepLink.parse: first src, form decoding, fragments ignored, missing src ignored. */
export function parseDeepLink(raw: string): PendingImportLink | null {
  const text = raw.trim();
  if (!text.startsWith("legado://") && !text.startsWith("yuedu://") && !text.startsWith("legado-rs://")) return null;
  if (text.length > 4 * 1024 * 1024) throw new Error("导入链接过长。");
  const link = new URL(text);
  let kind: DeepLinkKind;
  let parameter = "src";
  if (link.protocol === "legado-rs:") {
    if (link.username || link.password || link.port || link.hash || (link.pathname && link.pathname !== "/")) {
      throw new Error("不支持这个导入链接。");
    }
    if (link.hostname === "import-source") kind = "sources";
    else if (link.hostname === "import-rules") kind = "replacementRules";
    else throw new Error("不支持这个导入链接。");
    parameter = "url";
  } else {
    switch (link.pathname) {
      case "/bookSource": case "/rssSource": kind = "sources"; break;
      case "/replaceRule": kind = "replacementRules"; break;
      case "/textTocRule": kind = "txtTocRules"; break;
      case "/httpTTS": kind = "httpTtsConfigs"; break;
      case "/read": kind = "read"; break;
      case "/addToBookshelf": kind = "addToBookshelf"; break;
      case "/theme": kind = "theme"; break;
      case "/dictRule": kind = "dictRule"; break;
      case "/readConfig": kind = "readConfig"; break;
      case "/importonline":
        kind = link.hostname === "booksource" || link.hostname === "rsssource" ? "sources"
          : link.hostname === "replace" ? "replacementRules" : "auto";
        break;
      default: kind = "auto";
    }
  }
  const src = link.searchParams.get(parameter);
  if (!src) return null;
  return { kind, url: src };
}

/** Shared by the pasted-link entry and native deep-link delivery. */
export function parseImportInput(raw: string): PendingImportLink {
  const text = raw.trim();
  if (!text) throw new Error("请先输入链接或 JSON。");
  if (text.length > 4 * 1024 * 1024) throw new Error("导入内容过长。");
  const deepLink = parseDeepLink(text);
  if (deepLink) return deepLink;
  if (text.startsWith("{") || text.startsWith("[")) return { kind: "auto", url: text };
  const url = new URL(text);
  if (!/^https?:$/.test(url.protocol) || url.username || url.password) {
    throw new Error("请输入 HTTP(S) 地址、原版导入链接或 JSON。");
  }
  return { kind: "auto", url: text };
}
