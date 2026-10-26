import { computed, nextTick, onBeforeUnmount, ref, watch, type Ref } from "vue";

/** Keep fixed-height rows in an existing scroll container without copying their data. */
export function useVirtualList<T>(items: Readonly<Ref<readonly T[]>>, container: Ref<HTMLElement | null>, rowHeight: number) {
  const list = ref<HTMLElement | null>(null);
  const offset = ref(0), height = ref(600);
  let frame = 0;
  let observer: ResizeObserver | undefined;
  let attached: HTMLElement | null = null;
  function measure(): void {
    frame = 0;
    // 隐藏时保留原窗口，重新显示的第一帧即可沿用滚动位置。
    if (!container.value || !list.value || !container.value.clientHeight) return;
    offset.value = Math.max(0, container.value.getBoundingClientRect().top - list.value.getBoundingClientRect().top);
    height.value = container.value.clientHeight;
  }
  function schedule(): void { if (!frame) frame = requestAnimationFrame(measure); }
  const range = computed(() => {
    const count = items.value.length, visible = Math.ceil(height.value / rowHeight);
    const start = Math.max(0, Math.min(Math.floor(offset.value / rowHeight) - 6, count - visible - 6));
    const end = Math.min(count, start + visible + 12);
    return { start, end, top: start * rowHeight, bottom: (count - end) * rowHeight };
  });
  const visibleItems = computed(() => items.value.slice(range.value.start, range.value.end));
  watch([container, list], () => {
    attached?.removeEventListener("scroll", schedule);
    observer?.disconnect();
    attached = container.value;
    attached?.addEventListener("scroll", schedule, { passive: true });
    if (attached && typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(schedule);
      observer.observe(attached);
    }
    schedule();
  }, { flush: "post" });
  watch(items, schedule, { flush: "post" });
  function scrollToIndex(index: number, behavior: ScrollBehavior = "instant"): void {
    const viewport = container.value;
    if (!viewport || !list.value || index < 0) return;
    const top = viewport.scrollTop + list.value.getBoundingClientRect().top - viewport.getBoundingClientRect().top
      + index * rowHeight - Math.max(0, (viewport.clientHeight - rowHeight) / 2);
    viewport.scrollTo({ top: Math.max(0, top), behavior });
    measure();
  }
  async function reset(): Promise<void> {
    if (container.value) container.value.scrollTop = 0;
    offset.value = 0;
    await nextTick();
    measure();
  }
  onBeforeUnmount(() => {
    attached?.removeEventListener("scroll", schedule);
    observer?.disconnect();
    cancelAnimationFrame(frame);
  });
  return { list, range, visibleItems, scrollToIndex, reset };
}
