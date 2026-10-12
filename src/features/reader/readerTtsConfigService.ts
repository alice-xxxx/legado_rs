let selectConfigWriter: ((id: string) => void) | null = null;

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
