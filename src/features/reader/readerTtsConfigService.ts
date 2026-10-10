import { ref } from "vue";
import type { HttpTtsConfigMetadata } from "../../api/types";

export const readerHttpTtsConfigs = ref<HttpTtsConfigMetadata[]>([]);

let refreshGeneration = 0;
let selectConfigWriter: ((id: string) => void) | null = null;

export function beginReaderHttpTtsConfigRefresh(): number {
  refreshGeneration += 1;
  return refreshGeneration;
}

export function publishReaderHttpTtsConfigs(
  generation: number,
  configs: HttpTtsConfigMetadata[],
): boolean {
  if (generation !== refreshGeneration) return false;
  readerHttpTtsConfigs.value = configs;
  return true;
}

export function registerReaderTtsConfigSelectionWriter(writer: (id: string) => void): () => void {
  selectConfigWriter = writer;
  return () => {
    if (selectConfigWriter === writer) selectConfigWriter = null;
  };
}

export function selectReaderHttpTtsConfig(id: string): void {
  if (!selectConfigWriter) throw new Error("阅读器 TTS 设置尚未就绪。");
  selectConfigWriter(id);
}
