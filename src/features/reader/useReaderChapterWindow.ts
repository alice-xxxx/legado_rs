import { computed, nextTick, onScopeDispose, ref, watch, type Ref } from "vue";
import type { BookResource } from "../../api/types";
import { readTextResource } from "../../api/resources";
import { settings } from "../settings/settingsStore";
import type { CachedChapterContent } from "./useReaderChapterLoading";
import type { ReaderTextChapter } from "./readerContent";
import { readerPageWidth } from "./readerPosition";

/** A bounded DOM window over Rust-owned chapter resources, shared by scrolling and page turns. */
export function useReaderChapterWindow(options: {
  book: Ref<BookResource | null>; index: Ref<number>; content: Ref<CachedChapterContent | null>;
  visible: Ref<boolean>; busy: Ref<boolean>; viewport: Ref<HTMLElement | null>;
  adopt: (chapter: ReaderTextChapter) => void;
  animate: (viewport: HTMLElement, direction: -1 | 1, done: (committed: boolean) => void) => void;
  measure: () => void; setPage: (index: number) => void;
  save: () => Promise<void>; notify: (message: string, kind?: "success" | "error") => void;
}) {
  const chapters = ref<ReaderTextChapter[]>([]);
  const previews = new Map<number, HTMLElement>();
  const pending = new Map<number, Promise<ReaderTextChapter | null>>();
  let generation = 0;
  let controller = new AbortController();
  let turning = false;
  const enabled = computed(() => options.visible.value
    && (options.content.value?.kind === "text" || options.content.value?.kind === "richText"));
  const scrolling = computed(() => enabled.value && settings.value.reader.readingMode === "vertical");
  const ordered = () => options.book.value?.chapters.filter(chapter => !chapter.volume) ?? [];
  const adjacent = (direction: -1 | 1) => {
    const list = ordered(), position = list.findIndex(chapter => chapter.index === options.index.value);
    return list[position + direction];
  };
  function reset(): void {
    generation++; controller.abort(); controller = new AbortController();
    chapters.value = []; pending.clear(); previews.clear(); turning = false;
  }
  onScopeDispose(() => { generation++; controller.abort(); });
  watch([() => options.book.value?.id, options.visible, options.busy], ([id, visible, busy], [previousId]) => {
    if (id !== previousId || !visible || busy) reset();
  }, { flush: "sync" });
  async function prepare(index: number): Promise<ReaderTextChapter | null> {
    const cached = chapters.value.find(chapter => chapter.index === index);
    if (cached) return cached;
    const existing = pending.get(index);
    if (existing) return existing;
    const book = options.book.value, epoch = generation, signal = controller.signal;
    const entry = book?.chapters.find(chapter => chapter.index === index && !chapter.volume);
    if (!book || !entry) return null;
    const task = (async () => {
      const descriptor = entry.resource;
      if (!descriptor || !["text", "richText"].includes(descriptor.contentFormat)) return null;
      const body = await readTextResource(descriptor, signal);
      if (generation !== epoch || signal.aborted || options.book.value?.id !== book.id) return null;
      const list = ordered(), position = list.findIndex(chapter => chapter.index === options.index.value);
      if (!list.slice(Math.max(0, position - 1), position + 2).some(chapter => chapter.index === index)) return null;
      const chapter: ReaderTextChapter = { index, title: entry.title, content: descriptor.contentFormat === "richText"
        ? { schemaVersion: 1, kind: "richText", markup: body } : { schemaVersion: 1, kind: "text", text: body } };
      chapters.value = [...chapters.value.filter(item => item.index !== index), chapter].sort((a, b) => a.index - b.index);
      return chapter;
    })().catch(() => null).finally(() => { if (generation === epoch) pending.delete(index); });
    pending.set(index, task);
    return task;
  }
  watch([options.content, options.index, () => options.book.value?.chapters, () => settings.value.reader.readingMode], () => {
    if (!enabled.value || options.busy.value) return;
    const content = options.content.value;
    if (content?.kind !== "text" && content?.kind !== "richText") return;
    const index = options.index.value, list = ordered(), position = list.findIndex(chapter => chapter.index === index);
    const entry = list[position];
    if (!entry) return;
    if (!chapters.value.some(chapter => chapter.index === index && chapter.content === content)) {
      chapters.value = [...chapters.value.filter(chapter => chapter.index !== index), { index, title: entry.title, content }]
        .sort((a, b) => a.index - b.index);
    }
    const window = list.slice(Math.max(0, position - 1), position + 2);
    const retained = chapters.value.filter(chapter => window.some(entry => entry.index === chapter.index));
    if (retained.length !== chapters.value.length) chapters.value = retained;
    for (const chapter of window) void prepare(chapter.index);
  }, { immediate: true });
  const continuousChapters = computed(() => {
    if (!scrolling.value) return undefined;
    const list = ordered(), position = list.findIndex(chapter => chapter.index === options.index.value);
    let first = position, last = position;
    const has = (index: number) => chapters.value.some(chapter => chapter.index === list[index]?.index);
    while (first > 0 && has(first - 1)) first--;
    while (last + 1 < list.length && has(last + 1)) last++;
    return chapters.value.filter(chapter => list.slice(first, last + 1).some(entry => entry.index === chapter.index));
  });
  const previewChapters = computed(() => enabled.value && !scrolling.value
    ? chapters.value.filter(chapter => chapter.index !== options.index.value) : []);
  function onScroll(): void {
    const viewport = options.viewport.value;
    if (!scrolling.value || !viewport || options.busy.value) return;
    const top = viewport.getBoundingClientRect().top;
    const articles = [...viewport.querySelectorAll<HTMLElement>(".chapter-content[data-reader-chapter]")];
    const above = articles.filter(article => article.getBoundingClientRect().top <= top + 1);
    const article = above[above.length - 1] ?? articles[0];
    const chapter = chapters.value.find(chapter => chapter.index === Number(article?.dataset.readerChapter));
    if (!chapter || chapter.index === options.index.value) return;
    viewport.dataset.readerChapterIndex = String(chapter.index);
    options.adopt(chapter);
    void options.save();
  }
  function commit(chapter: ReaderTextChapter, direction: -1 | 1): void {
    const viewport = options.viewport.value, epoch = generation;
    if (!viewport) return;
    options.adopt(chapter);
    void nextTick().then(() => {
      if (generation !== epoch || options.viewport.value !== viewport) return;
      options.measure();
      const count = Math.max(1, Math.ceil((viewport.scrollWidth - 1) / readerPageWidth(viewport)));
      options.setPage(direction > 0 ? 0 : count - 1);
      turning = false;
      void options.save();
    });
  }
  function chapterPreview(direction: -1 | 1): HTMLElement | null {
    const target = adjacent(direction);
    return enabled.value && !scrolling.value && target ? previews.get(target.index) ?? null : null;
  }
  function commitChapterTurn(direction: -1 | 1): void {
    const target = adjacent(direction);
    const chapter = chapters.value.find(chapter => chapter.index === target?.index);
    if (chapter) commit(chapter, direction);
  }
  function waitForChapter(index: number, mode: string): Promise<ReaderTextChapter | null> {
    const signal = controller.signal;
    return new Promise(resolve => {
      let stop = () => {}, settled = false;
      const finish = (chapter: ReaderTextChapter | null) => {
        if (settled) return;
        settled = true; clearTimeout(timer); stop(); signal.removeEventListener("abort", abort); resolve(chapter);
      };
      const abort = () => finish(null);
      const timer = setTimeout(abort, 15_000);
      signal.addEventListener("abort", abort, { once: true });
      stop = watch([() => options.book.value?.chapters.find(chapter => chapter.index === index)?.resource?.src,
        () => settings.value.reader.readingMode], ([src, currentMode]) => {
        if (currentMode !== mode) finish(null);
        else if (src) void prepare(index).then(finish);
      });
      if (signal.aborted) abort();
      else if (options.book.value?.chapters.find(chapter => chapter.index === index)?.resource) void prepare(index).then(finish);
    });
  }
  function turn(direction: -1 | 1): boolean {
    if (!enabled.value) return false;
    const viewport = options.viewport.value;
    if (!viewport) return false;
    if (scrolling.value) {
      viewport.scrollBy({ top: direction * viewport.clientHeight, behavior: "smooth" });
      return true;
    }
    if (direction > 0 ? viewport.scrollLeft + readerPageWidth(viewport) < viewport.scrollWidth - 1 : viewport.scrollLeft > 1) return false;
    const target = adjacent(direction);
    if (!target) return false;
    if (turning) return true;
    turning = true;
    const epoch = generation, mode = settings.value.reader.readingMode;
    void prepare(target.index).then(async prepared => {
      const chapter = prepared ?? (!target.resource ? await waitForChapter(target.index, mode) : null);
      await nextTick();
      const preview = previews.get(target.index);
      if (generation !== epoch || settings.value.reader.readingMode !== mode || !chapter || !preview) {
        if (generation === epoch) {
          turning = false;
          if (settings.value.reader.readingMode === mode) options.notify("相邻章节暂未准备好，请重试。", "error");
        }
        return;
      }
      options.animate(preview, direction, committed => {
        if (generation !== epoch) return;
        if (!committed) { turning = false; return; }
        commit(chapter, direction);
      });
    });
    return true;
  }
  return { continuousChapters, previewChapters, onScroll, turn, chapterPreview, commitChapterTurn, prepare,
    chapterElement: (index: number) => options.viewport.value?.querySelector<HTMLElement>(`.chapter-content[data-reader-chapter="${index}"]`)
      ?? previews.get(index)?.querySelector<HTMLElement>(`.chapter-content[data-reader-chapter="${index}"]`),
    setPreview: (index: number, element: HTMLElement | null) => { if (element) previews.set(index, element); else previews.delete(index); } };
}
