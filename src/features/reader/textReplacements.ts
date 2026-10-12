import type { DisplayReplacementRule } from "../../api/types";

/** Apply display rules to the original text; never write the result to cache. */
export function applyTextRules(text: string, rules: readonly DisplayReplacementRule[], bookId?: string): string {
  for (const rule of rules) {
    if (!rule.enabled || !rule.pattern || (rule.scope !== "all" && rule.scope !== `book:${bookId}`)) continue;
    if (!rule.isRegex) {
      text = text.split(rule.pattern).join(rule.replacement);
      continue;
    }
    try {
      const regex = new RegExp(rule.pattern, "g");
      if (!rule.replacement.startsWith("@js:")) {
        text = text.replace(regex, rule.replacement);
      } else {
        const evaluate = new Function("result", `let R, z; return eval(${JSON.stringify(rule.replacement.slice(4))});`) as (result: string) => unknown;
        text = text.replace(regex, match => {
          try { return String(evaluate(match) ?? ""); }
          catch { return match; }
        });
      }
    } catch {
      // Invalid rules keep their input unchanged, as in the existing reader.
    }
  }
  return text;
}

/** Rich text retains its markup; rules apply only to text nodes. */
export function applyMarkupRules(markup: string, rules: readonly DisplayReplacementRule[], bookId?: string): string {
  const active = rules.some(rule => rule.enabled && rule.pattern && (rule.scope === "all" || rule.scope === 'book:' + bookId));
  if (!active && !/<(?:link|style)\b/i.test(markup)) return markup;
  const root = document.createElement("div");
  root.innerHTML = markup;
  root.querySelectorAll("link,style").forEach(node => node.remove());
  if (active) {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode as Text;
      node.data = applyTextRules(node.data, rules, bookId);
    }
  }
  return root.innerHTML;
}
