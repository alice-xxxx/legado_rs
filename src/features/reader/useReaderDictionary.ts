import { ref, type Ref } from "vue";
import type { AppSettingsResource, BookResource, DisplayReplacementRule, DictionaryLanguage } from "../../api/types";
import { lookupDictionary } from "../../api/reading";
import { readTextResource } from "../../api/resources";
import { makeDisplayHtml } from "./displayHtml";

type ReadonlyRef<T> = Readonly<Ref<T>>;

interface ReaderDictionaryOptions {
  frame: ReadonlyRef<HTMLIFrameElement | null>;
  book: ReadonlyRef<BookResource | null>;
  chapter: ReadonlyRef<BookResource["chapters"][number] | null>;
  visible: ReadonlyRef<boolean>;
  settings: ReadonlyRef<AppSettingsResource>;
  replacementRules: ReadonlyRef<DisplayReplacementRule[]>;
  getReaderLoadRevision: () => number;
  getRestoreRevision: () => number;
  isBackupRestoreInProgress: () => boolean;
  errorText: (error: unknown) => string;
}

/** 管理阅读页的选词、词典查询结果和过期请求。 */
export function useReaderDictionary(options: ReaderDictionaryOptions) {
  const dictionaryOpen = ref(false);
  const dictionaryWord = ref("");
  const dictionaryLanguage = ref<DictionaryLanguage>("en");
  const dictionaryBusy = ref(false);
  const dictionaryError = ref("");
  const dictionaryTitle = ref("");
  const dictionaryProvider = ref("");
  const dictionarySourceUrl = ref("");
  const dictionaryHtml = ref("");
  let lookupGeneration = 0;

  function selectedReaderText(): string {
    return options.frame.value?.contentDocument?.getSelection()?.toString().trim() ?? "";
  }

  function openReaderDictionary(): void {
    lookupGeneration += 1;
    dictionaryOpen.value = true;
    dictionaryError.value = "";
    const selected = selectedReaderText();
    if (selected) dictionaryWord.value = Array.from(selected).slice(0, 128).join("");
  }

  function useSelectedReaderText(): void {
    const selected = selectedReaderText();
    if (!selected) {
      dictionaryError.value = "请先在章节内容中选中要查的词，或直接输入词语。";
      return;
    }
    dictionaryWord.value = Array.from(selected).slice(0, 128).join("");
    dictionaryError.value = "";
  }

  async function lookupReaderDictionary(): Promise<void> {
    const word = dictionaryWord.value.trim();
    if (dictionaryBusy.value) return;
    if (!word) {
      dictionaryError.value = "请输入要查询的词语。";
      return;
    }
    const bookId = options.book.value?.id;
    const chapterId = options.chapter.value?.id;
    if (!options.visible.value || !dictionaryOpen.value || !bookId || !chapterId) return;
    const language = dictionaryLanguage.value;
    const readerLoadRevisionAtStart = options.getReaderLoadRevision();
    const restoreRevisionAtStart = options.getRestoreRevision();
    const generation = ++lookupGeneration;
    const isCurrentLookup = () => generation === lookupGeneration
      && restoreRevisionAtStart === options.getRestoreRevision()
      && readerLoadRevisionAtStart === options.getReaderLoadRevision()
      && options.visible.value
      && dictionaryOpen.value
      && options.book.value?.id === bookId
      && options.chapter.value?.id === chapterId
      && dictionaryWord.value.trim() === word
      && dictionaryLanguage.value === language
      && !options.isBackupRestoreInProgress();
    dictionaryBusy.value = true;
    dictionaryError.value = "";
    dictionaryTitle.value = "";
    dictionaryProvider.value = "";
    dictionarySourceUrl.value = "";
    dictionaryHtml.value = "";
    try {
      const result = await lookupDictionary(word, language);
      if (!isCurrentLookup()) return;
      const rawHtml = await readTextResource(result.resource);
      if (!isCurrentLookup()) return;
      dictionaryTitle.value = result.title;
      dictionaryProvider.value = result.provider;
      dictionarySourceUrl.value = result.sourceUrl;
      dictionaryHtml.value = makeDisplayHtml(rawHtml, options.settings.value.reader, options.replacementRules.value);
    } catch (error) {
      if (isCurrentLookup()) dictionaryError.value = options.errorText(error);
    } finally {
      if (generation === lookupGeneration) dictionaryBusy.value = false;
    }
  }

  function closeReaderDictionary(): void {
    lookupGeneration += 1;
    dictionaryOpen.value = false;
    dictionaryBusy.value = false;
    dictionaryError.value = "";
    dictionaryTitle.value = "";
    dictionaryProvider.value = "";
    dictionarySourceUrl.value = "";
    dictionaryHtml.value = "";
  }

  return {
    dictionaryOpen,
    dictionaryWord,
    dictionaryLanguage,
    dictionaryBusy,
    dictionaryError,
    dictionaryTitle,
    dictionaryProvider,
    dictionarySourceUrl,
    dictionaryHtml,
    selectedReaderText,
    openReaderDictionary,
    useSelectedReaderText,
    lookupReaderDictionary,
    closeReaderDictionary,
  };
}
