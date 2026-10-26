import type { ConfigurationImportKind } from "../../api/settings";
export type ImportKind = ConfigurationImportKind;
export type PendingImportLink = { kind: ImportKind | "auto"; url: string };

export const deepLinkLabels: Record<PendingImportLink["kind"], string> = {
  sources: "源", replacementRules: "替换规则", txtTocRules: "TXT 目录规则",
  httpTtsConfigs: "HTTP TTS", auto: "配置",
};

/** 只接收四类配置；兼容链接取第一个 src 参数，缺失时忽略。 */
export function parseDeepLink(raw: string): PendingImportLink | null {
  const text = raw.trim();
  if (!/^(foliver|legado|yuedu):\/\//.test(text)) return null;
  if (text.length > 4 * 1024 * 1024) throw new Error("导入链接过长。");
  const link = new URL(text);
  let kind: ImportKind;
  let parameter = "src";
  if (link.protocol === "foliver:") {
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
      case "/importonline":
        if (link.hostname === "booksource" || link.hostname === "rsssource") kind = "sources";
        else if (link.hostname === "replace") kind = "replacementRules";
        else return null;
        break;
      default: return null;
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
