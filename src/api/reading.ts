import { command } from "./ipc";
import type { DictionaryLanguage, DictionaryLookupResult } from "./types";

export const lookupDictionary = (word: string, language: DictionaryLanguage) =>
  command<DictionaryLookupResult>("lookup_dictionary", { word, language });
