/// <reference types="vite/client" />

declare module "konsta/material-colors" {
  export default function materialColors(color: string): Record<string, string>;
}

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}
