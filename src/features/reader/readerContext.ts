import type { useAppReaderFeatures } from "./useAppReaderFeatures";
import type { BookSourceSwitchContract } from "../sources/sourceContext";
export type ReaderContextFeatures = ReturnType<typeof useAppReaderFeatures>;
export interface ReaderPageProps {
  readerFeatures: ReaderContextFeatures;
  closeReader: (discardProgress?: boolean) => Promise<void>;
  bookSourceSwitch: BookSourceSwitchContract;
  shelfBatchRecoveryRequired: boolean;
}
