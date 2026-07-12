import { homedir } from "node:os";
import { join } from "node:path";
import { createServer, type ViteDevServer } from "vite";

let devServer: ViteDevServer | undefined;

const startupCommandResults = {
  thinclaw_get_status: { setup_completed: false, dev_mode_wizard: true },
  get_user_config: {},
  get_system_specs: {
    total_memory: 16_000_000_000,
    used_memory: 4_000_000_000,
    cpu_brand: "E2E virtual CPU",
    cpu_usage: 0,
    cpu_cores: 8,
    platform: "browser-e2e",
    app_memory: 0,
    memory_bandwidth_gbps: 0,
  },
  direct_runtime_snapshot: {
    kind: "none",
    displayName: "No local runtime",
    readiness: "unavailable",
    capabilities: [],
    supportedCapabilities: [],
    exposurePolicy: "direct_only",
    unavailableReason: "Browser E2E fixture",
  },
  direct_runtime_get_active_engine_info: {
    id: "none",
    display_name: "No local engine",
    available: false,
    requires_setup: false,
    description: "Browser E2E fixture",
    hf_tag: "",
    single_file_model: true,
  },
  direct_runtime_get_engine_setup_status: {
    needs_setup: false,
    setup_in_progress: false,
    message: "Browser E2E fixture",
  },
  get_permission_status: { accessibility: true, screen_recording: true },
  list_models: [],
};

const startupFixtureScript = `
  window.__wdio_mocks__ ??= {};
  const fixtures = ${JSON.stringify(startupCommandResults)};
  for (const [command, result] of Object.entries(fixtures)) {
    window.__wdio_mocks__[command] = async () => structuredClone(result);
  }
  window.__TAURI_INTERNALS__.metadata ??= {};
  window.__TAURI_INTERNALS__.metadata.currentWindow ??= { label: "main" };
  window.__TAURI_INTERNALS__.metadata.currentWebview ??= { label: "main" };
`;

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./e2e/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: [
    {
      browserName: "tauri",
      // WebdriverIO's default macOS temp cache can be cleaned/quarantined
      // between download and launch. A normal user cache is stable locally
      // and remains ephemeral on CI runners.
      "wdio:chromedriverOptions": {
        cacheDir: join(homedir(), ".cache", "thinclaw-webdriver"),
      },
    },
  ],
  logLevel: "error",
  bail: 1,
  waitforTimeout: 10_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 1,
  services: [
    [
      "@wdio/tauri-service",
      {
        mode: "browser",
        devServerUrl: "http://127.0.0.1:1420",
        logLevel: "error",
      },
    ],
  ],
  onPrepare: async () => {
    devServer = await createServer({
      root: "frontend",
      plugins: [
        {
          name: "thinclaw-e2e-startup-fixtures",
          transformIndexHtml: {
            order: "pre",
            handler: () => [
              {
                tag: "script",
                children: startupFixtureScript,
                injectTo: "head-prepend",
              },
            ],
          },
        },
      ],
      server: { host: "127.0.0.1", port: 1420, strictPort: true },
    });
    await devServer.listen();
  },
  onComplete: async () => {
    await devServer?.close();
    devServer = undefined;
  },
  framework: "jasmine",
  reporters: ["spec"],
  jasmineOpts: {
    defaultTimeoutInterval: 30_000,
  },
};
