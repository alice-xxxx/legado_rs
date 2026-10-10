import { command } from "./ipc";
import type { ResourceDescriptor, BatchRemoveBooksResponse, ShelfSortKey, ShelfSortOrder } from "./types";

export const batchSetBookGroups = (bookIds: string[], groups: string[]) =>
  command<ResourceDescriptor>("batch_set_book_groups", { bookIds, groups });

export const batchRemoveBooks = (bookIds: string[]) =>
  command<BatchRemoveBooksResponse>("batch_remove_books", { bookIds });

export const renameShelfGroup = (oldName: string, newName: string) =>
  command<ResourceDescriptor>("rename_shelf_group", { oldName, newName });

export const deleteShelfGroup = (groupName: string) =>
  command<ResourceDescriptor>("delete_shelf_group", { groupName });

export const setShelfSort = (key: ShelfSortKey, order: ShelfSortOrder) =>
  command<ResourceDescriptor>("set_shelf_sort", { key, order });

export const setShelfOrder = (orderedBookIds: string[]) =>
  command<ResourceDescriptor>("set_shelf_order", { orderedBookIds });

export const createShelfGroup = (groupName: string) =>
  command<ResourceDescriptor>("create_shelf_group", { groupName });
