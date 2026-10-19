import { modifyDatabase } from "./feDb";

export const clearFinishedTasks = () => modifyDatabase([{ action: "delete", table: "tasks", filter: {} }]);
