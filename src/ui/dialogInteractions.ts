const layers = ".modal-backdrop,.overlay-backdrop,[data-back-layer]";
const controls = "button:not(:disabled),a[href],input:not(:disabled):not([type=hidden]),select:not(:disabled),textarea:not(:disabled),[tabindex]:not([tabindex='-1'])";
const visible = (element: HTMLElement) => element.isConnected && element.getClientRects().length > 0
  && getComputedStyle(element).visibility !== "hidden" && !element.closest("[inert]");

/** Shared keyboard, focus and scroll behavior for dialogs across all feature hosts. */
export function installDialogInteractions(): () => void {
  const active = new Map<HTMLElement, HTMLElement | null>();
  const openers: HTMLElement[] = [];
  let disposed = false;
  let sequence = 0;
  function top(): HTMLElement | undefined { return [...active.keys()].reverse().find(visible); }
  function sync(): void {
    if (disposed) return;
    const current = [...document.querySelectorAll<HTMLElement>(layers)].filter(visible);
    const removed = [...active.entries()].filter(([layer]) => !current.includes(layer)).reverse();
    for (const [layer] of removed) active.delete(layer);
    for (const layer of current) {
      if (active.has(layer)) continue;
      const focused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const returnFocus = focused && focused !== document.body && visible(focused) && !layer.contains(focused)
        ? focused : [...openers].reverse().find(element => visible(element) && !layer.contains(element)) ?? null;
      active.set(layer, returnFocus);
      const panel = layer.querySelector<HTMLElement>("[role=dialog],.app-modal") ?? layer.firstElementChild as HTMLElement | null;
      if (panel) {
        panel.setAttribute("role", "dialog");
        panel.setAttribute("aria-modal", "true");
        if (!panel.hasAttribute("aria-labelledby") && !panel.hasAttribute("aria-label")) {
          const title = panel.querySelector<HTMLElement>("h1,h2,h3,header strong,.popover-title");
          if (title) { title.id ||= `app-dialog-title-${++sequence}`; panel.setAttribute("aria-labelledby", title.id); }
        }
        panel.tabIndex = -1;
      }
      if (!layer.contains(document.activeElement)) {
        panel?.focus({ preventScroll: true });
      }
    }
    document.body.classList.toggle("app-dialog-open", current.length > 0);
    const layer = top();
    for (const [, target] of removed) {
      if (target && !target.matches("input,textarea,select,[contenteditable]") && visible(target) && (!layer || layer.contains(target))) { target.focus({ preventScroll: true }); break; }
    }
  }
  const observer = new MutationObserver(records => {
    const relevant = records.some(record => record.type === "attributes"
      ? record.target instanceof Element && record.target.matches(layers)
      : [...record.addedNodes, ...record.removedNodes].some(node => node instanceof Element
        && (node.matches(layers) || node.querySelector(layers))));
    if (relevant) sync();
  });
  const rememberOpener = (event: PointerEvent) => {
    const opener = event.target instanceof Element ? event.target.closest<HTMLElement>("button,a,input,select,textarea,[tabindex]") : null;
    if (opener) { openers.push(opener); if (openers.length > 8) openers.shift(); }
  };
  const keydown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.isComposing) return;
    const layer = top();
    if (!layer) return;
    if (event.key === "Escape") {
      event.preventDefault(); event.stopPropagation();
      [...layer.querySelectorAll<HTMLButtonElement>("button[data-back-action]")].find(button => visible(button) && !button.disabled)?.click();
    } else if (event.key === "Tab") {
      const targets = [...layer.querySelectorAll<HTMLElement>(controls)].filter(visible);
      const first = targets[0], last = targets[targets.length - 1];
      if (!first || !last) { event.preventDefault(); return; }
      if (!layer.contains(document.activeElement) || (event.shiftKey ? document.activeElement === first : document.activeElement === last)) {
        event.preventDefault(); (event.shiftKey ? last : first).focus({ preventScroll: true });
      }
    }
  };
  observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["hidden", "style"] });
  document.addEventListener("pointerdown", rememberOpener, true);
  document.addEventListener("keydown", keydown);
  sync();
  return () => {
    disposed = true;
    observer.disconnect();
    document.removeEventListener("pointerdown", rememberOpener, true);
    document.removeEventListener("keydown", keydown);
    document.body.classList.remove("app-dialog-open");
    active.clear();
  };
}
