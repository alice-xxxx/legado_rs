import { command } from "./ipc";
import type { DiscoveryResponse, HomeConfigDocument, ResourceDescriptor } from "./types";

export const listDiscoveryCategories = (sourceId: string) =>
  command<DiscoveryResponse>("list_discovery_categories", { sourceId });

export const listDiscoveryBooks = (sourceId: string, categoryId: string, page = 1) =>
  command<DiscoveryResponse>("list_discovery_books", { sourceId, categoryId, page });

export const getHomeConfig = () => command<ResourceDescriptor>("get_home_config");

export const saveHomeConfig = (config: HomeConfigDocument) =>
  command<ResourceDescriptor>("save_home_config", { config });
