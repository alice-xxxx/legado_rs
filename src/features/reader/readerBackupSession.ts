import type { Ref } from "vue";
import type { BookResource } from "../../api/types";

/** Reader capabilities required by the backup feature, without exposing reader UI state. */
export interface ReaderBackupSession {
  readingBook: Ref<BookResource | null>;
  readerProgressDirty: Ref<boolean>;
  hasActiveReaderMedia: () => boolean;
  pauseReaderMediaAndSave: () => Promise<boolean>;
  saveCurrentProgress: () => Promise<void>;
  closeReaderAfterRestore: () => Promise<void>;
}

let activeSession: ReaderBackupSession | null = null;

export function installReaderBackupSession(session: ReaderBackupSession): () => void {
  activeSession = session;
  return () => {
    if (activeSession === session) activeSession = null;
  };
}

export function getReaderBackupSession(): ReaderBackupSession {
  if (!activeSession) throw new Error("Reader backup session is not mounted");
  return activeSession;
}
