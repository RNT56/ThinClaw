import { homedir } from "node:os";
import { join } from "node:path";
import { createServer, type ViteDevServer } from "vite";

let devServer: ViteDevServer | undefined;

const desktopStatus = {
  engine_running: true,
  engine_connected: true,
  slack_enabled: false,
  telegram_enabled: false,
  port: 18789,
  gateway_mode: "local",
  remote_url: null,
  remote_token: null,
  device_id: "browser-e2e-device",
  auth_token: "browser-e2e-token",
  state_dir: "/tmp/thinclaw-browser-e2e",
  has_huggingface_token: false,
  huggingface_granted: false,
  has_anthropic_key: false,
  anthropic_granted: false,
  has_brave_key: false,
  brave_granted: false,
  has_openai_key: false,
  openai_granted: false,
  has_openrouter_key: false,
  openrouter_granted: false,
  has_gemini_key: false,
  gemini_granted: false,
  has_groq_key: false,
  groq_granted: false,
  custom_secrets: [],
  allow_local_tools: true,
  workspace_mode: "default",
  workspace_root: null,
  local_inference_enabled: false,
  selected_cloud_brain: null,
  selected_cloud_model: null,
  profiles: [],
  setup_completed: true,
  auto_start_gateway: false,
  dev_mode_wizard: false,
  auto_approve_tools: false,
  bootstrap_completed: true,
  custom_llm_url: null,
  custom_llm_key: null,
  custom_llm_model: null,
  custom_llm_enabled: false,
  enabled_cloud_providers: [],
  enabled_cloud_models: {},
  has_xai_key: false,
  xai_granted: false,
  has_venice_key: false,
  venice_granted: false,
  has_together_key: false,
  together_granted: false,
  has_moonshot_key: false,
  moonshot_granted: false,
  has_minimax_key: false,
  minimax_granted: false,
  has_nvidia_key: false,
  nvidia_granted: false,
  has_qianfan_key: false,
  qianfan_granted: false,
  has_mistral_key: false,
  mistral_granted: false,
  has_xiaomi_key: false,
  xiaomi_granted: false,
  has_cohere_key: false,
  cohere_granted: false,
  has_voyage_key: false,
  voyage_granted: false,
  has_deepgram_key: false,
  deepgram_granted: false,
  has_elevenlabs_key: false,
  elevenlabs_granted: false,
  has_stability_key: false,
  stability_granted: false,
  has_fal_key: false,
  fal_granted: false,
  has_bedrock_key: false,
  bedrock_granted: false,
};

const startupCommandResults = {
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
  direct_runtime_get_sidecar_status: {
    chat_running: false,
    stt_running: false,
    image_configured: false,
  },
  direct_history_get_conversations: [],
  list_projects: [],
  direct_inference_discover_cloud_models: {
    providers: [],
    totalModels: 0,
    errors: [],
  },
  direct_runtime_discover_hf_models: [],
  check_missing_standard_assets: [],
  get_permission_status: { accessibility: true, screen_recording: true },
  list_models: [],
  thinclaw_check_bootstrap_needed: false,
  thinclaw_get_sessions: {
    sessions: [
      {
        session_key: "agent:main",
        title: "Main",
        updated_at_ms: 1_700_000_000_000,
        source: "browser-e2e",
      },
    ],
  },
  thinclaw_channels_list: { channels: [] },
  thinclaw_config_get: { settings: [] },
  thinclaw_gmail_status: {
    enabled: false,
    configured: false,
    status: "not_configured",
    project_id: "",
    subscription_id: "",
    label_filters: [],
    allowed_senders: [],
    missing_fields: [],
    oauth_configured: false,
  },
  thinclaw_cron_list: [],
  thinclaw_jobs_list: { jobs: [], capabilities: {}, unavailable: {} },
  thinclaw_jobs_summary: {
    total: 0,
    pending: 0,
    in_progress: 0,
    completed: 0,
    failed: 0,
    cancelled: 0,
    interrupted: 0,
    stuck: 0,
  },
};

const startupFixtureScript = `
  window.__wdio_mocks__ ??= {};
  const fixtures = ${JSON.stringify(startupCommandResults)};
  for (const [command, result] of Object.entries(fixtures)) {
    window.__wdio_mocks__[command] = async () => structuredClone(result);
  }
  const desktopStatus = ${JSON.stringify(desktopStatus)};
  window.__wdio_mocks__.thinclaw_get_status = async () => {
    const setupComplete = localStorage.getItem("__thinclaw_e2e_setup_complete") === "true";
    return structuredClone(setupComplete
      ? desktopStatus
      : { ...desktopStatus, setup_completed: false, dev_mode_wizard: true });
  };
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
    random: false,
  },
};
