<script setup lang="ts">
import ReaderPage from "./ReaderPage.vue";
import type { ReaderPageProps } from "./readerContext";
const props = defineProps<ReaderPageProps>();
const reader = props.readerFeatures;
import PdfReaderPage from "./PdfReaderPage.vue";
import { readingBook } from "./readerBookState";
import { readBook } from "../../api/database";
import { modifyDatabase } from "../../api/feDb";

// 导入阶段不解析 PDF；阅读器打开后才保存实际页数。
let saved = "";
async function savePageCount(count: number): Promise<void> {
  reader.readerPageCount.value = 1;
  const book = readingBook.value;
  if (!book || saved === `${book.id}:${count}`) return;
  saved = `${book.id}:${count}`;
  try {
    await modifyDatabase([{ operation: "saveBook", bookId: book.id, changes: { pageCount: count } }], { wait: true });
    const updated = await readBook(book.id);
    if (readingBook.value?.id === book.id) readingBook.value = updated;
  } catch (error) { saved = ""; console.error(error); }
}
</script>
<template>
  <ReaderPage v-bind="props">
    <PdfReaderPage v-if="reader.readingPdfPage.value" :src="reader.readingPdfPage.value.src"
      :page-index="reader.readingPdfPage.value.pageIndex" :default-zoom="reader.activePdfZoom.value"
      :find-open="reader.readerControlsOpen.value && reader.readerControlsMode.value === 'search'"
      @update:find-open="reader.readerControlsOpen.value = $event; reader.readerControlsMode.value = 'search'"
      @loaded="savePageCount" @page-selected="reader.selectReaderChapter" @close-reader="props.closeReader()" />
  </ReaderPage>
</template>
