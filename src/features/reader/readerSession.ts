import type { Ref } from "vue";
import type { Bookmark, BookResource } from "../../api/types";

/** Small set of reader operations used by screens that start or leave reading. */
export interface ReaderSession {
  readerVisible: Ref<boolean>;
  readerBusy: Readonly<Ref<boolean>>;
  readingBook: Ref<BookResource | null>;
  readerProgressDirty: Ref<boolean>;
  startReading: (book: BookResource, chapterIndex?: number, offset?: number) => Promise<void>;
  saveCurrentProgress: () => Promise<void>;
  finishReadingSession: () => Promise<void>;
  closeReader: (discardProgress?: boolean) => Promise<void>;
  openBookmark: (bookmark: Bookmark) => Promise<void>;
}

let activeSession: ReaderSession | null = null;

export function installReaderSession(session: ReaderSession): () => void {
  activeSession = session;
  return () => {
    if (activeSession === session) activeSession = null;
  };
}

export function getReaderSession(): ReaderSession {
  if (!activeSession) throw new Error("Reader session is not mounted");
  return activeSession;
}
