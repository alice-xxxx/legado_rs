import type { Ref } from "vue";
import type { LocalImportFormat } from "../../api/books";

/** Narrow bridge for the app-owned importer shared by shelf and external-file workflows. */
export interface LocalBookImportSession {
  localBookImportBusy: Readonly<Ref<boolean>>;
  importLocalBook: (format?: LocalImportFormat) => Promise<void>;
  requestLinkImport: (text: string) => void;
}

let activeSession: LocalBookImportSession | null = null;

export function installLocalBookImportSession(session: LocalBookImportSession): () => void {
  activeSession = session;
  return () => {
    if (activeSession === session) activeSession = null;
  };
}

export function getLocalBookImportSession(): LocalBookImportSession {
  if (!activeSession) throw new Error("Local book import session is not installed");
  return activeSession;
}
