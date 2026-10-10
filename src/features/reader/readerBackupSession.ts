import type { Ref } from "vue";
import type { AppSettingsResource, BookResource } from "../../api/types";

/** Reader capabilities required by the backup feature, without exposing reader UI state. */
export interface ReaderBackupSession {
  settings: Ref<AppSettingsResource>;
  saveSettingsBusy: Ref<boolean>;
  pendingSettingsSave: Ref<ReturnType<typeof setTimeout> | null>;
  readingBook: Ref<BookResource | null>;
  readerProgressDirty: Ref<boolean>;
  hasActiveReaderMedia: () => boolean;
  pauseReaderMediaAndSave: () => Promise<boolean>;
  saveCurrentProgress: () => Promise<void>;
  persistSettings: () => Promise<boolean>;
  scheduleSettingsSave: () => void;
  closeReaderAfterRestore: () => Promise<void>;
  deferSettingsSaveDuringRestore: () => boolean;
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
