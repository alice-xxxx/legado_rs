import { defineConfig } from "vite";
import type { Plugin } from "vite";
import vue from "@vitejs/plugin-vue";
// @ts-expect-error type error without @types/node package
import process from "node:process";
// @ts-expect-error type error without @types/node package
import { spawn } from "node:child_process";
// @ts-expect-error type error without @types/node package
import { fileURLToPath } from "node:url";
const host = process.env.TAURI_DEV_HOST;

function vueTypecheckWatch(): Plugin {
  return {
    name: "vue-typecheck-watch",
    apply: "serve",
    configureServer(server) {
      const checkerPath = fileURLToPath(
        new URL("./node_modules/vue-tsc/bin/vue-tsc.js", import.meta.url),
      );
      const checker = spawn(
        process.execPath,
        [checkerPath, "--noEmit", "--watch"],
        { cwd: process.cwd(), stdio: "inherit" },
      );
      let stopped = false;
      const stop = () => {
        if (stopped) return;
        stopped = true;
        process.removeListener("exit", stop);
        if (checker.exitCode === null) checker.kill();
      };

      process.once("exit", stop);
      server.httpServer?.once("close", stop);
      server.config.logger.info("Starting frontend type watcher: vue-tsc --noEmit --watch");
      checker.once("error", (error: Error) => {
        server.config.logger.error(`Could not start frontend type watcher: ${error.message}`);
      });
      checker.once("exit", (code: number | null, signal: string | null) => {
        if (!stopped) {
          server.config.logger.error(
            `Frontend type watcher exited unexpectedly (code ${code}, signal ${signal ?? "none"})`,
          );
        }
      });
    },
  };
}

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [vue(), vueTypecheckWatch()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // The iOS deployment target is 15.0; keep emitted syntax compatible with its WKWebView.
  build: {
    target: "safari15",
    // Split runtime and media libraries into separate real chunks, rather than
    // hiding Vite's large-chunk warning by increasing its threshold.
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            { name: "vue-runtime", test: /[\\/]node_modules[\\/](?:vue|@vue|vue-router)[\\/]/, priority: 20 },
            { name: "hls-player", test: /[\\/]node_modules[\\/]hls\.js[\\/]/, priority: 15 },
            { name: "pdfjs-engine", test: /[\\/]node_modules[\\/]pdfjs-dist[\\/]/, priority: 15 },
            { name: "compat-runtime", test: /[\\/]node_modules[\\/]core-js[\\/]/, priority: 15 },
          ],
        },
      },
    },
  },
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
