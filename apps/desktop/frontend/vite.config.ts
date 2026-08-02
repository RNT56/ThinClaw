import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Keep the initial Desktop shell below the enforced 500 KiB chunk budget.
  // These are stable third-party boundaries; route-level Cockpit panels remain
  // lazy imports and are not folded back into the entry chunk.
  build: {
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            { name: "react-vendor", test: /node_modules\/(?:react|react-dom|scheduler)\//, minSize: 0 },
            { name: "motion-vendor", test: /node_modules\/(?:framer-motion|motion-dom)\//, minSize: 0 },
            { name: "tauri-vendor", test: /node_modules\/@tauri-apps\//, minSize: 0 },
            { name: "dialog-vendor", test: /node_modules\/(?:@radix-ui|react-remove-scroll|aria-hidden|use-callback-ref|use-sidecar)\//, minSize: 0 },
          ],
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
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
      // 3. tell Vite to ignore watching `backend`
      ignored: ["**/backend/**"],
    },
  },
}));
