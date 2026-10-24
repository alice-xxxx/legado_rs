import Swiper from "swiper";
import { EffectCreative, EffectFade, EffectFlip } from "swiper/modules";
import { PageFlip, FlipCorner, FlippingState, type Point } from "@gullabs/flipbook-core";
import type { AnimatedReadingMode } from "../settings/readerModes";
import "swiper/css";
import "swiper/css/effect-creative";
import "swiper/css/effect-fade";
import "swiper/css/effect-flip";
import "@gullabs/flipbook-core/style.css";

export type PageTurnDirection = -1 | 1;
export interface ReaderPageTurn {
  readonly axis: "x" | "y";
  readonly size: number;
  readonly progress: number;
  update(progress: number, point: Point): void;
  settle(commit: boolean, done: (committed: boolean) => void): void;
  finish?(): void;
  dispose(): void;
}

/** Library-owned rendering on two inert pages; the live viewport never moves during a drag. */
export function createReaderPageTurn(viewport: HTMLElement, direction: PageTurnDirection,
  mode: AnimatedReadingMode, targetPage: number, touchStart?: Point, targetViewport = viewport): ReaderPageTurn | null {
  const frame = viewport.parentElement;
  const view = viewport.ownerDocument.defaultView;
  const { width, height } = viewport.getBoundingClientRect();
  if (!frame?.classList.contains("reader-content-frame") || !view || width <= 0 || height <= 0) return null;
  const layer = viewport.ownerDocument.createElement("div");
  layer.className = `reader-page-turn-layer reader-page-turn-${mode}`;
  layer.setAttribute("aria-hidden", "true");
  layer.setAttribute("inert", "");
  frame.append(layer);
  const style = view.getComputedStyle(viewport);
  layer.style.backgroundColor = style.backgroundColor;
  const axis = mode === "slide-vertical" ? "y" : "x";
  const size = axis === "y" ? height : width;
  function snapshot(source: HTMLElement, left: number): HTMLElement {
    const leaf = viewport.ownerDocument.createElement("div");
    const window = viewport.ownerDocument.createElement("div");
    window.className = "reader-page-turn-snapshot";
    window.style.backgroundColor = style.backgroundColor;
    window.style.setProperty("--novel-background-image", style.getPropertyValue("--novel-background-image"));
    window.style.setProperty("--novel-background-opacity", style.getPropertyValue("--novel-background-opacity"));
    const page = source.cloneNode(true) as HTMLElement;
    page.classList.remove("reader-chapter-preview");
    for (const node of [page, ...page.querySelectorAll<HTMLElement>("*")]) {
      node.removeAttribute("id");
      node.removeAttribute("contenteditable");
      for (const attribute of Array.from(node.attributes)) {
        if (attribute.name.toLowerCase().startsWith("on")) node.removeAttribute(attribute.name);
      }
      if (node.matches("a,button,input,select,textarea,[tabindex]")) node.setAttribute("tabindex", "-1");
    }
    page.querySelectorAll("audio,video,iframe,object,embed").forEach(node => node.remove());
    page.classList.add("reader-page-turn-content");
    // PageFlip clones the page face. CSS offsets survive cloneNode; scrollLeft does not.
    page.style.setProperty("--snapshot-left", `${-left}px`);
    page.style.setProperty("--snapshot-width", `${width}px`);
    page.style.setProperty("--snapshot-height", `${height}px`);
    window.append(page);
    leaf.append(window);
    return leaf;
  }
  const current = snapshot(viewport, viewport.scrollLeft);
  const destination = snapshot(targetViewport, targetPage * width);
  const leaves = direction > 0 ? [current, destination] : [destination, current];
  const initial = direction > 0 ? 0 : 1;
  const target = 1 - initial;
  let progress = 0;
  let disposed = false;
  let pending: ((committed: boolean) => void) | null = null;
  const complete = (committed: boolean) => {
    const callback = pending;
    // A completion can be emitted inside the library's draw/dispatch stack.
    // Let it finish before the host disposes the renderer and commits the reading page.
    if (callback) queueMicrotask(() => {
      if (!disposed && pending === callback) { pending = null; callback(committed); }
    });
  };
  const reducedMotion = () => view.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (mode !== "simulation") {
    let commitOnFinish = false;
    layer.classList.add("swiper");
    const wrapper = viewport.ownerDocument.createElement("div");
    wrapper.className = "swiper-wrapper";
    leaves.forEach(leaf => { leaf.className = "swiper-slide"; wrapper.append(leaf); });
    layer.append(wrapper);
    const swiper = new Swiper(layer, {
      modules: mode === "cover" ? [EffectCreative] : mode === "fade" ? [EffectFade] : mode === "flip" ? [EffectFlip] : [],
      effect: mode === "cover" ? "creative" : mode === "slide-vertical" ? "slide" : mode,
      direction: axis === "y" ? "vertical" : "horizontal", initialSlide: initial,
      width, height, speed: 300, allowTouchMove: false, simulateTouch: false,
      resizeObserver: false, updateOnWindowResize: false,
      creativeEffect: { perspective: false, prev: { translate: ["-100%", 0, 0], shadow: true }, next: { translate: [0, 0, 0] } },
      // Fade text out before showing the next page to avoid overlapping lines.
      fadeEffect: { mode: "out-in" },
      on: {
        setTranslate(instance) {
          // Cover pages keep their physical stack order on both sides of the midpoint.
          if (mode === "cover") instance.slides.forEach((leaf, index) => { leaf.style.zIndex = String(2 - index); });
        },
        transitionEnd(instance) { complete(instance.activeIndex === target); },
      },
    });
    return {
      axis, size, get progress() { return progress; },
      update(value) {
        if (disposed) return;
        progress = Math.max(0, Math.min(1, value));
        swiper.setProgress(initial + direction * progress, 0);
      },
      settle(commit, done) {
        if (disposed) return;
        commitOnFinish = commit;
        pending = done;
        // A click creates and turns the layer in the same task. Flush its
        // initial position so the browser actually starts the CSS transition.
        void wrapper.offsetWidth;
        const moved = swiper.slideTo(commit ? target : initial, reducedMotion() ? 0 : 300);
        if (!moved && !swiper.animating) complete(commit);
      },
      finish() {
        const callback = pending;
        pending = null;
        callback?.(commitOnFinish);
      },
      dispose() { disposed = true; pending = null; swiper.destroy(true, true); layer.remove(); },
    };
  }
  const engine = new PageFlip(layer, {
    width, height, initialPage: initial, sizing: "fixed", autoSize: false,
    usePortrait: true, pointerInput: [], flipOnClick: "never", foldCornerOnHover: false,
    flippingTime: reducedMotion() ? 1 : 650, pageBackground: style.backgroundColor, injectStyles: false,
  });
  engine.loadFromHTML(leaves);
  let dragging = false;
  const start = { x: direction > 0 ? width - 1 : 1, y: touchStart?.y ?? height * .8 };
  let last = start;
  engine.on("changeState", event => {
    if (event.data.state === FlippingState.READ) complete(engine.getCurrentPageIndex() === target);
  });
  engine.on("turnRejected", () => complete(false));
  return {
    axis, size, get progress() { return progress; },
    update(value, point) {
      if (disposed) return;
      progress = Math.max(0, Math.min(1, value));
      if (!dragging) {
        dragging = true;
        engine.startUserTouch(start);
        // Select the known navigation direction before the finger crosses the centre.
        engine.userMove({ x: direction > 0 ? width - 8 : 8, y: start.y }, true);
      }
      // The library folds across an open book (two page widths). Map the
      // single-page reader's drag onto that span, including its halfway point.
      last = { x: direction > 0 ? width * (1 - 2 * progress) : width * 2 * progress, y: point.y };
      engine.userMove(last, true);
    },
    settle(commit, done) {
      if (disposed) return;
      pending = done;
      if (!commit) { engine.cancelTurn(); complete(false); }
      else if (dragging) {
        engine.userStop(last);
        if (engine.getState() === FlippingState.READ) complete(engine.getCurrentPageIndex() === target);
      } else {
        const corner = start.y < height / 2 ? FlipCorner.TOP : FlipCorner.BOTTOM;
        if (!(direction > 0 ? engine.flipNext(corner) : engine.flipPrev(corner))) complete(false);
      }
    },
    dispose() { disposed = true; pending = null; engine.destroy(); layer.remove(); },
  };
}
