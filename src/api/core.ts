import { command } from "./ipc";
import type { AppBootstrap } from "./types";

export const appBootstrap = () => command<AppBootstrap>("app_bootstrap");
