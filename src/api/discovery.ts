import { command } from "./ipc";
import type { DiscoveryResponse } from "./types";

export const listDiscoveryCategories = (sourceId: string) =>
  command<DiscoveryResponse>("list_discovery_categories", { sourceId });

export const listDiscoveryBooks = (sourceId: string, categoryId: string, page = 1) =>
  command<DiscoveryResponse>("list_discovery_books", { sourceId, categoryId, page });
