// 解析阅读器使用的章节资源并生成安全的显示 HTML。
import type { ChapterMediaResource, ChapterPdfPageResource, ChapterRichTextResource, DisplayReplacementRule, ReaderSettings } from "../../api/types";

export interface PdfPageResource { src: string; pageIndex: number; defaultZoom: "page-fit" | "page-width" | "actual-size" }
export interface ReaderDisplayOptions { imageOnly?: boolean; audioOnly?: boolean; videoOnly?: boolean; comicScaleMode?: "fit-width" | "actual-size"; chapterTitle?: string }

/** Convert processed plain-text content into inert display markup in the UI layer. */
export function plainTextToDisplayHtml(text: string): string {
  const document = documentForDisplay();
  const normalized = text.replace(/\r\n?/g, "\n");
  const blocks = normalized.split(/\n{2,}/);
  for (const block of blocks) {
    const paragraph = document.createElement("p");
    const lines = block.split("\n");
    for (let index = 0; index < lines.length; index += 1) {
      if (index > 0) paragraph.append(document.createElement("br"));
      paragraph.append(document.createTextNode(lines[index]));
    }
    document.body.append(paragraph);
  }
  return `<!doctype html>${document.documentElement.outerHTML}`;
}

function documentForDisplay(): Document {
  return document.implementation.createHTMLDocument("");
}

/** Convert Rust-sanitized rich-text data into an inert document in the UI layer. */
export function richTextResourceToDisplayHtml(resource: ChapterRichTextResource): string {
  if (resource.kind !== "richText" || typeof resource.markup !== "string") {
    throw new Error("富文本章节资源格式无效。");
  }
  const document = new DOMParser().parseFromString(resource.markup, "text/html");
  document.querySelectorAll("script, iframe, object, embed, form, base, meta, link").forEach((node) => node.remove());
  document.querySelectorAll("*").forEach((node) => {
    for (const attribute of [...node.attributes]) {
      const value = attribute.value.trim();
      if (/^on/i.test(attribute.name)
        || ((attribute.name === "href" || attribute.name === "src" || attribute.name === "poster")
          && /^javascript:/i.test(value))) {
        node.removeAttribute(attribute.name);
      }
    }
  });
  return `<!doctype html>${document.documentElement.outerHTML}`;
}

export function mediaResourceToDisplayHtml(resource: ChapterMediaResource): string {
  const document = documentForDisplay();
  if (resource.kind !== "media"
    || !["audio", "video"].includes(resource.mediaType)
    || !["direct", "hls"].includes(resource.format)
    || !resource.src) {
    throw new Error("媒体章节资源格式无效。");
  }

  const section = document.createElement("section");
  section.className = resource.mediaType === "audio" ? "audio-chapter" : "video-chapter";
  const media = document.createElement(resource.mediaType);
  media.controls = true;
  media.preload = "metadata";
  media.setAttribute("playsinline", "");
  media.src = resource.src;
  media.dataset.legadoMediaFormat = resource.format;

  if (resource.mediaType === "audio") {
    const source = document.createElement("source");
    source.src = resource.src;
    source.dataset.legadoMediaFormat = resource.format;
    media.append(source);
  } else {
    for (const option of resource.sources ?? []) {
      const source = document.createElement("source");
      if (option.src) source.src = option.src;
      source.dataset.legadoMediaLabel = option.label;
      if (option.format) source.dataset.legadoMediaFormat = option.format;
      if (option.selected) source.dataset.legadoMediaSelected = "true";
      if (option.unavailableReason) source.dataset.legadoMediaUnavailable = option.unavailableReason;
      media.append(source);
    }
  }

  media.append(document.createTextNode(
    resource.mediaType === "audio" ? "当前环境不支持音频播放。" : "当前环境不支持视频播放。",
  ));
  section.append(media);
  document.body.append(section);
  return `<!doctype html>${document.documentElement.outerHTML}`;
}

export function typedPdfPageResource(
  resource: ChapterPdfPageResource,
): PdfPageResource | null {
  if (resource.kind !== "pdfPage"
    || !Number.isInteger(resource.pageIndex)
    || resource.pageIndex < 0
    || !["page-fit", "page-width", "actual-size"].includes(resource.defaultZoom)) {
    return null;
  }

  // Rust materializes stable resource refs before this JSON reaches the
  // WebView. Treat src as the final consumable resource instead of deriving it
  // from the chapter descriptor URL or Resource Runtime path layout.
  let assetUrl: URL;
  try {
    assetUrl = new URL(resource.src);
  } catch {
    return null;
  }
  if (!["http:", "https:"].includes(assetUrl.protocol)
    || assetUrl.username || assetUrl.password) {
    return null;
  }
  return {
    src: assetUrl.href,
    pageIndex: resource.pageIndex,
    defaultZoom: resource.defaultZoom,
  };
}

export function isImageOnlyChapter(raw: string): boolean {
  const document = new DOMParser().parseFromString(raw, "text/html");
  if (!document.body.querySelector("img")) return false;
  const textOnly = document.body.cloneNode(true) as HTMLElement;
  textOnly.querySelectorAll("img, picture, svg, video, audio, style, script, br, hr").forEach((node) => node.remove());
  return !textOnly.textContent?.trim();
}

export function makeDisplayHtml(raw: string, reader: ReaderSettings, replacements: DisplayReplacementRule[], bookId?: string, options: ReaderDisplayOptions = {}, showBackgroundImage = false): string {
  const document = new DOMParser().parseFromString(raw, "text/html");
  document.querySelectorAll("script, iframe, object, embed, form, base, meta[http-equiv='refresh']").forEach((node) => node.remove());
  document.querySelectorAll("meta[name='viewport']").forEach((node) => node.remove());
  const viewport = document.createElement("meta");
  viewport.name = "viewport";
  viewport.content = "width=device-width, initial-scale=1, minimum-scale=1, maximum-scale=1, user-scalable=no";
  document.head.append(viewport);
  document.querySelectorAll("meta[http-equiv], link").forEach((node) => {
    if (node instanceof HTMLMetaElement) {
      const directive = node.httpEquiv.toLocaleLowerCase();
      if (directive === "content-security-policy" || directive === "refresh") node.remove();
    } else if (node instanceof HTMLLinkElement && !node.relList.contains("stylesheet")) {
      node.remove();
    }
  });
  document.querySelectorAll("*").forEach((node) => {
    for (const attribute of [...node.attributes]) {
      if (/^on/i.test(attribute.name) || ((attribute.name === "href" || attribute.name === "src") && /^\s*javascript:/i.test(attribute.value))) {
        node.removeAttribute(attribute.name);
      }
    }
  });
  applyReaderReplacements(document.body, replacements, bookId);
  const chapterTitle = options.chapterTitle?.trim();
  if (chapterTitle && !options.imageOnly && !options.audioOnly && !options.videoOnly) {
    removeLeadingDuplicateChapterTitle(document.body, chapterTitle);
    const heading = document.createElement("h1");
    heading.dataset.legadoChapterTitle = "true";
    heading.dataset.testid = "reader-chapter-title";
    heading.textContent = chapterTitle;
    document.body.prepend(heading);
  }

  document.head.prepend(makeReaderCsp(document));
  const style = document.createElement("style");
  style.dataset.legadoReaderOverride = "true";
  style.textContent = options.audioOnly ? `
    html,body{width:100%;height:auto;min-height:100%;margin:0!important;overflow:auto!important;}
    body{box-sizing:border-box!important;min-height:100vh!important;padding:24px!important;background:${safeCssColor(reader.backgroundColor, "#f7f3e9")}!important;
      color:${safeCssColor(reader.textColor, "#3f3b34")}!important;font-family:${cssFont(reader.fontFamily)}!important;
      font-size:${clamp(reader.fontSizePx,12,36)}px!important;line-height:${clamp(reader.lineHeight,1.1,3)}!important;
      column-width:auto!important;column-gap:0!important;column-fill:auto!important;}
    body *{box-sizing:border-box!important;max-width:100%!important;font-family:inherit!important;}
    audio{display:block!important;width:min(100%,720px)!important;max-width:100%!important;margin:24px auto!important;}
  ` : options.videoOnly ? `
    html,body{width:100%;height:100%;min-height:100%;margin:0!important;overflow:hidden!important;}
    body{box-sizing:border-box!important;height:100vh!important;min-height:100%!important;padding:12px!important;
      display:grid!important;place-items:center!important;background:${safeCssColor(reader.backgroundColor, "#17191c")}!important;}
    body *{box-sizing:border-box!important;max-width:100%!important;}
    video{display:block!important;width:100%!important;max-height:100%!important;background:#000!important;object-fit:contain!important;}
  ` : options.imageOnly ? `
    html,body{width:100%;height:auto;min-height:100%;margin:0!important;overflow-x:${options.comicScaleMode === "actual-size" ? "auto" : "hidden"}!important;overflow-y:auto!important;}
    body{box-sizing:border-box!important;min-height:100vh!important;padding:0!important;background:${safeCssColor(reader.backgroundColor, "#f7f3e9")}!important;
      color:${safeCssColor(reader.textColor, "#3f3b34")}!important;column-width:auto!important;column-gap:0!important;column-fill:auto!important;
      font-family:${cssFont(reader.fontFamily)}!important;font-size:${clamp(reader.fontSizePx,12,36)}px!important;
      line-height:${clamp(reader.lineHeight,1.1,3)}!important;text-align:center!important;scrollbar-width:thin!important;}
    body *{box-sizing:border-box!important;max-width:none!important;line-height:normal!important;font-family:inherit!important;}
    body img{display:block!important;width:${options.comicScaleMode === "actual-size" ? "auto" : "100%"}!important;
      max-width:${options.comicScaleMode === "actual-size" ? "none" : "100%"}!important;height:auto!important;margin:0 auto!important;object-fit:contain!important;}
  ` : reader.verticalScroll ? `
    html,body{width:100%;height:auto!important;min-height:100%;margin:0!important;}
    html{overflow-x:hidden!important;overflow-y:auto!important;}
    body{--reader-gutter:min(8vw,96px);box-sizing:border-box!important;min-height:100vh!important;padding:0 var(--reader-gutter)!important;
      background:${safeCssColor(reader.backgroundColor, "#f7f3e9")}!important;color:${safeCssColor(reader.textColor, "#3f3b34")}!important;
      column-width:auto!important;column-gap:0!important;column-fill:auto!important;
      font-family:${cssFont(reader.fontFamily)}!important;font-size:${clamp(reader.fontSizePx,12,36)}px!important;
      line-height:${clamp(reader.lineHeight,1.1,3)}!important;text-align:${safeTextAlign(reader.textAlign)}!important;overflow:visible!important;}
    body *{max-width:100%!important;line-height:${clamp(reader.lineHeight,1.1,3)}!important;
      font-family:${cssFont(reader.fontFamily)}!important;font-size:inherit!important;}
    p,li,blockquote{break-inside:auto!important;white-space:normal!important;}
    img,video,svg{max-width:100%!important;height:auto!important;object-fit:contain!important;}
    a{color:inherit!important;}
  ` : `
    html,body{width:100%;height:100%;min-height:100%;margin:0!important;overflow:hidden!important;}
    body{--reader-gutter:min(8vw,96px);box-sizing:border-box!important;height:100vh!important;padding:0 var(--reader-gutter)!important;
      background:${safeCssColor(reader.backgroundColor, "#f7f3e9")}!important;color:${safeCssColor(reader.textColor, "#3f3b34")}!important;
      column-width:calc(100vw - min(16vw,192px))!important;column-gap:min(16vw,192px)!important;
      column-fill:auto!important;overflow-x:auto!important;overflow-y:hidden!important;
      font-family:${cssFont(reader.fontFamily)}!important;font-size:${clamp(reader.fontSizePx,12,36)}px!important;
      line-height:${clamp(reader.lineHeight,1.1,3)}!important;text-align:${safeTextAlign(reader.textAlign)}!important;scrollbar-width:none!important;}
    body::-webkit-scrollbar{display:none!important;}
    body *{max-width:100%!important;line-height:${clamp(reader.lineHeight,1.1,3)}!important;
      font-family:${cssFont(reader.fontFamily)}!important;font-size:inherit!important;}
    p,li,blockquote{break-inside:auto!important;white-space:normal!important;}
    img,video,svg{max-width:100%!important;height:auto!important;object-fit:contain!important;}
    a{color:inherit!important;}
  `;
  style.textContent = `@media (pointer: coarse) { html, body { touch-action: pan-x pan-y !important; } }\n${style.textContent}`;
  if (chapterTitle && !options.imageOnly && !options.audioOnly && !options.videoOnly) {
    style.textContent += `
      body [data-legado-chapter-title]{display:block!important;margin:0 0 1.2em!important;
        font-size:${clamp(reader.fontSizePx * 1.35, 18, 36)}px!important;line-height:1.4!important;
        font-weight:700!important;text-align:left!important;break-inside:avoid!important;break-after:avoid!important;}
    `;
  }
  const backgroundImage = safeCssImageUrl(reader.backgroundImageSrc);
  const backgroundImageOpacity = clamp(reader.backgroundImageOpacity, 0, 100) / 100;
  if (showBackgroundImage && backgroundImage !== "none" && backgroundImageOpacity > 0
      && !options.audioOnly && !options.videoOnly && !options.imageOnly) {
    const backgroundColor = safeCssColor(reader.backgroundColor, "#f7f3e9");
    style.textContent += `
      body{background-color:${backgroundColor}!important;isolation:isolate!important;}
      body::before{content:"";position:fixed;inset:0;z-index:-1;pointer-events:none;
        background-image:${backgroundImage}!important;background-size:cover!important;
        background-position:center!important;background-repeat:no-repeat!important;
        opacity:${backgroundImageOpacity}!important;}
    `;
  }
  (document.head ?? document.documentElement).append(style);
  return `<!doctype html>${document.documentElement.outerHTML}`;
}

/** 判断处理后的章节是否含有可见正文，避免空内容被当作成功加载。 */
export function hasDisplayableReaderContent(html: string): boolean {
  if (!html.trim()) return false;

  const body = new DOMParser().parseFromString(html, "text/html").body;
  body.querySelector("[data-legado-chapter-title]")?.remove();
  body.querySelectorAll("style, script, template, noscript, [hidden], [aria-hidden='true']").forEach((node) => node.remove());

  const hiddenStyle = /(?:^|;)\s*(?:display\s*:\s*none|visibility\s*:\s*hidden|content-visibility\s*:\s*hidden|opacity\s*:\s*0(?:\.0*)?)\s*(?:;|$)/i;
  body.querySelectorAll<HTMLElement>("[style]").forEach((element) => {
    if (hiddenStyle.test(element.getAttribute("style") ?? "")) element.remove();
  });

  if ((body.textContent ?? "").replace(/[\s\u00a0\u200b\uFEFF]/g, "").length > 0) return true;

  const hasSource = (selector: string): boolean => Array.from(body.querySelectorAll<HTMLElement>(selector))
    .some((element) => Boolean(element.getAttribute("src")?.trim()));
  if (hasSource("img[src]") || hasSource("audio[src], audio source[src], video[src], video source[src]")) return true;
  if (body.querySelector("svg path[d], svg circle[cx], svg ellipse[cx], svg polygon[points], svg polyline[points], svg rect[width], svg image[href]")) return true;
  if (body.querySelector("[data-legado-document='pdf-page']")) return true;

  return Array.from(body.querySelectorAll<HTMLElement>("[style]")).some((element) =>
    /url\(\s*(['"]?)[^\s'"\)]+\1\s*\)/i.test(element.getAttribute("style") ?? ""));
}

function makeReaderCsp(document: Document): HTMLMetaElement {
  // Rust has already resolved rich-text asset URLs before publication.
  const allowedAssets = "http: https: data: blob:";
  const policy = document.createElement("meta");
  policy.httpEquiv = "Content-Security-Policy";
  policy.content = [
    "default-src 'none'",
    `img-src ${allowedAssets}`,
    `media-src ${allowedAssets}`,
    "style-src 'unsafe-inline' http: https:",
    "font-src http: https: data:",
    "script-src 'none'",
    "connect-src 'none'",
    "frame-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
  return policy;
}

function safeCssColor(value: string, fallback: string): string {
  return /^#[\da-f]{3,8}$/i.test(value) || /^(rgb|rgba|hsl|hsla)\([\d\s.,%+-]+\)$/i.test(value) ? value : fallback;
}

function safeCssImageUrl(value?: string): string {
  if (!value) return "none";
  try {
    const url = new URL(value);
    if ((url.protocol !== "http:" && url.protocol !== "https:") || url.username || url.password) return "none";
    return `url("${url.href.replace(/["\\\r\n]/g, "\\function applyReaderReplacements(")}")`;
  } catch {
    return "none";
  }
}

function safeTextAlign(value: string): "left" | "right" | "center" | "justify" {
  return ["left", "right", "center", "justify"].includes(value)
    ? value as "left" | "right" | "center" | "justify"
    : "justify";
}

function cssFont(font: string): string {
  const fonts: Record<string, string> = {
    serif: "Georgia, 'Noto Serif', 'Songti SC', serif",
    sans: "system-ui, -apple-system, 'Segoe UI', sans-serif",
    system: "system-ui, -apple-system, 'Segoe UI', sans-serif",
    mono: "ui-monospace, 'SFMono-Regular', monospace",
  };
  return fonts[font] ?? "system-ui, sans-serif";
}

export function clamp(value: number, minimum: number, maximum: number): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.min(maximum, Math.max(minimum, number)) : minimum;
}

function applyReaderReplacements(root: HTMLElement, replacements: DisplayReplacementRule[], bookId?: string): void {
  const active = replacements.filter((rule) => rule.enabled && rule.pattern && (rule.scope === "all" || rule.scope === `book:${bookId}`));
  if (!active.length) return;
  const compiled = active.flatMap((rule) => {
    if (!rule.isRegex) {
      return [(content: string) => content.split(rule.pattern).join(rule.replacement)];
    }
    try {
      const regex = new RegExp(rule.pattern, "g");
      if (!rule.replacement.startsWith("@js:")) {
        return [(content: string) => content.replace(regex, rule.replacement)];
      }

      const script = rule.replacement.slice(4);
      const evaluate = new Function(
        "result",
        `let R, z; return eval(${JSON.stringify(script)});`,
      ) as (result: string) => unknown;
      return [(content: string) => content.replace(regex, (match) => {
        try {
          return String(evaluate(match) ?? "");
        } catch {
          return match;
        }
      })];
    } catch {
      // Invalid regular expressions or scripts leave their source text untouched.
      return [];
    }
  });
  if (!compiled.length) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = node.parentElement;
      return parent && !parent.closest("script,style,noscript,textarea") ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });
  const textNodes: Text[] = [];
  while (walker.nextNode()) textNodes.push(walker.currentNode as Text);
  for (const node of textNodes) {
    let content = node.data;
    for (const replace of compiled) content = replace(content);
    node.data = content;
  }
}

// 避免书源正文自带的同名开篇标题与阅读器标题重复。
function removeLeadingDuplicateChapterTitle(body: HTMLElement, chapterTitle: string): void {
  const normalize = (value: string) => value.replace(/\s+/g, "");
  const expected = normalize(chapterTitle);
  const first = Array.from(body.childNodes).find((node) => Boolean(node.textContent?.trim()));
  if (!first || !expected) return;

  if (first.nodeType === Node.TEXT_NODE) {
    const text = first.textContent ?? "";
    const firstLine = text.match(/^\s*([^\r\n]*)(?:\r?\n|$)/)?.[1] ?? "";
    if (normalize(firstLine) === expected) first.textContent = text.replace(/^\s*[^\r\n]*(?:\r?\n)?/, "");
    return;
  }

  if (!(first instanceof HTMLElement)) return;
  if (normalize(first.textContent ?? "") === expected) {
    first.remove();
    return;
  }
  const heading = first.matches("h1,h2,h3") ? first : first.querySelector("h1,h2,h3");
  if (heading && normalize(heading.textContent ?? "") === expected) heading.remove();
}
