import type { useAppReaderFeatures } from "./useAppReaderFeatures";
import type { BookSourceSwitchContract } from "../sources/sourceContext";
import type { SourceMetadata } from "../../api/types";
export type ReaderContextFeatures = ReturnType<typeof useAppReaderFeatures>;
export interface ReaderPageProps {
  readerFeatures: ReaderContextFeatures;
  closeReader: (discardProgress?: boolean) => Promise<void>;
  bookSourceSwitch: BookSourceSwitchContract;
  bookSourceCandidates: SourceMetadata[];
  sourceName: (id: string) => string;
  shelfBatchRecoveryRequired: boolean;
}
