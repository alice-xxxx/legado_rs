import { command } from "./ipc";
type HistoryMutation="saveBookmark"|"removeBookmark"|"recordSession"|"removeSearch"|"clearSearch";
export const mutateHistory=(kind:HistoryMutation,payload:unknown,requestId:number)=>
  command<{requestId:number}>("mutate_history",{kind,payload,requestId});
