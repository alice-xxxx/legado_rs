import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, watch, type Ref } from "vue";
import type { SearchBookResult } from "../../api/types";

export const SEARCH_RESULT_ROW_HEIGHT = 80;

export function searchResultRange(count: number, offset: number, viewportHeight: number, overscan = 6) {
  const first = Math.min(count, Math.max(0, Math.floor(offset / SEARCH_RESULT_ROW_HEIGHT)));
  const last = Math.min(count, Math.max(first, Math.ceil((offset + viewportHeight) / SEARCH_RESULT_ROW_HEIGHT)));
  const start = Math.max(0, first - overscan);
  const end = Math.min(count, last + overscan);
  return { start, end, top: start * SEARCH_RESULT_ROW_HEIGHT, bottom: (count - end) * SEARCH_RESULT_ROW_HEIGHT };
}

/** Retain every backend result, but mount only rows near the document viewport. */
export function useSearchResultWindow(results: Readonly<Ref<SearchBookResult[]>>, element: Ref<HTMLElement | null>) {
  const offset = ref(0);
  const viewportHeight = ref(window.innerHeight);
  const viewportWidth = ref(window.innerWidth);
  const range = computed(() => searchResultRange(results.value.length, offset.value, viewportHeight.value));
  const renderedResults = computed(() => results.value.slice(range.value.start, range.value.end));
  let active = false;
  let frame = 0;
  let observer: ResizeObserver | null = null;
  let remembered: { id?: string; withinRow: number; scrollY: number } | null = null;

  function measure(): void {
    frame = 0;
    if (!active) return;
    viewportHeight.value = window.innerHeight;
    viewportWidth.value = window.innerWidth;
    if (element.value) offset.value = -element.value.getBoundingClientRect().top;
    else offset.value = 0;
  }
  function scheduleMeasure(): void {
    if (active && !frame) frame = requestAnimationFrame(measure);
  }
  function observe(): void {
    observer?.disconnect();
    if (!active || !element.value || typeof ResizeObserver === "undefined") return;
    observer ??= new ResizeObserver(scheduleMeasure);
    observer.observe(element.value);
    const page = element.value.closest(".search-page-prototype");
    if (page) observer.observe(page);
  }
  function rememberPosition(): void {
    const top = element.value ? -element.value.getBoundingClientRect().top : 0;
    const index = Math.max(0, Math.min(results.value.length - 1, Math.floor(top / SEARCH_RESULT_ROW_HEIGHT)));
    remembered = { id: results.value[index]?.resultId,
      withinRow: top - index * SEARCH_RESULT_ROW_HEIGHT, scrollY: window.scrollY };
  }
  function scrollToStart(): void {
    remembered = null;
    offset.value = 0;
    if (element.value) {
      const header = document.querySelector<HTMLElement>(".search-prototype-topbar");
      window.scrollTo({ top: Math.max(0, window.scrollY + element.value.getBoundingClientRect().top
        - (header?.getBoundingClientRect().height ?? 0)), behavior: "instant" });
    }
    scheduleMeasure();
  }
  async function activate(): Promise<void> {
    if (!active) {
      active = true;
      window.addEventListener("scroll", scheduleMeasure, { capture: true, passive: true });
      window.addEventListener("resize", scheduleMeasure, { passive: true });
    }
    await nextTick();
    if (!active) return;
    observe();
    if (remembered) {
      const index = results.value.findIndex(result => result.resultId === remembered!.id);
      const top = index >= 0 && element.value
        ? window.scrollY + element.value.getBoundingClientRect().top
          + index * SEARCH_RESULT_ROW_HEIGHT + remembered.withinRow
        : remembered.scrollY;
      window.scrollTo({ top: Math.max(0, top), behavior: "instant" });
      remembered = null;
    }
    measure();
  }
  function suspend(): void {
    active = false;
    cancelAnimationFrame(frame);
    frame = 0;
    observer?.disconnect();
    window.removeEventListener("scroll", scheduleMeasure, true);
    window.removeEventListener("resize", scheduleMeasure);
  }
  onMounted(activate);
  onActivated(activate);
  onDeactivated(() => { if (!remembered) rememberPosition(); suspend(); });
  onBeforeUnmount(suspend);
  watch(element, () => { observe(); scheduleMeasure(); }, { flush: "post" });
  watch(results, scheduleMeasure, { flush: "post" });
  return { range, renderedResults, viewportWidth, rememberPosition,
    forgetPosition: () => { remembered = null; }, scrollToStart };
}
