/**
 * Deterministic Hugging Face browser fixture.
 *
 * The fixture lives in the same page realm as the Tauri IPC shim so it can
 * model genuinely asynchronous searches/downloads and emit native-style
 * progress events without touching the network.
 */

export const HF_E2E_REPOS = {
  chat: "acme/TinyChat-GGUF",
  vision: "acme/TinyVision-GGUF",
  gated: "acme/GatedChat-GGUF",
  stale: "acme/Slow-Stale-GGUF",
  latest: "acme/Latest-Result-GGUF",
} as const;

export const HF_E2E_ARTIFACTS = {
  chatSharded: "tinychat-q4-k-m",
  chatSingle: "tinychat-q8-0",
  visionSharded: "tinyvision-q4-k-m",
  projectorF16: "mmproj-f16",
  projectorQ5: "mmproj-q5-k-m",
  gated: "gated-q4-k-m",
} as const;

export const HF_E2E_DOWNLOAD_IDS = {
  chatSharded: "hf-e2e-tinychat-q4-k-m",
  chatSingle: "hf-e2e-tinychat-q8-0",
  visionSharded: "hf-e2e-tinyvision-q4-k-m",
  gated: "hf-e2e-gated-q4-k-m",
} as const;

const revision = "0123456789abcdef0123456789abcdef01234567";

function modelCard(
  id: string,
  options: { gated?: boolean; downloads?: number; likes?: number } = {},
) {
  return {
    id,
    author: id.split("/")[0],
    name: id.split("/")[1],
    downloads: options.downloads ?? 12_345,
    likes: options.likes ?? 321,
    tags: ["gguf", "text-generation"],
    last_modified: "2026-07-29T12:00:00Z",
    gated: options.gated ?? false,
    revision,
  };
}

function artifact(
  id: string,
  downloadId: string,
  label: string,
  files: Array<{ path: string; size: number; size_display: string }>,
  options: {
    primaryFile?: string;
    quantType?: string;
    isMmproj?: boolean;
  } = {},
) {
  const totalSize = files.reduce((sum, file) => sum + file.size, 0);
  return {
    id,
    download_id: downloadId,
    label,
    layout: "gguf_variants",
    files: files.map(file => ({ ...file, sha256: "a".repeat(64) })),
    primary_file: options.primaryFile ?? files[0]?.path ?? null,
    quant_type: options.quantType ?? null,
    is_mmproj: options.isMmproj ?? false,
    total_size: totalSize,
    total_size_display: `${(totalSize / 1_000_000_000).toFixed(1)} GB`,
  };
}

const chatShards = [
  {
    path: "tinychat-q4_k_m-00001-of-00002.gguf",
    size: 1_600_000_000,
    size_display: "1.6 GB",
  },
  {
    path: "tinychat-q4_k_m-00002-of-00002.gguf",
    size: 1_500_000_000,
    size_display: "1.5 GB",
  },
];

const visionShards = [
  {
    path: "tinyvision-q4_k_m-00001-of-00002.gguf",
    size: 2_000_000_000,
    size_display: "2.0 GB",
  },
  {
    path: "tinyvision-q4_k_m-00002-of-00002.gguf",
    size: 1_900_000_000,
    size_display: "1.9 GB",
  },
];

const plans = {
  [HF_E2E_REPOS.chat]: {
    repo_id: HF_E2E_REPOS.chat,
    revision,
    engine_id: "llamacpp",
    task: "chat",
    category: "LLM",
    format_tag: "gguf",
    layout: "gguf_variants",
    artifacts: [
      artifact(
        HF_E2E_ARTIFACTS.chatSharded,
        HF_E2E_DOWNLOAD_IDS.chatSharded,
        "Q4_K_M · balanced",
        chatShards,
        {
          primaryFile: chatShards[0].path,
          quantType: "Q4_K_M",
        },
      ),
      artifact(
        HF_E2E_ARTIFACTS.chatSingle,
        HF_E2E_DOWNLOAD_IDS.chatSingle,
        "Q8_0 · highest quality",
        [
          {
            path: "tinychat-q8_0.gguf",
            size: 5_700_000_000,
            size_display: "5.7 GB",
          },
        ],
        { quantType: "Q8_0" },
      ),
    ],
    companion_artifacts: [],
    warnings: ["The selected revision is immutable and verified before install."],
  },
  [HF_E2E_REPOS.vision]: {
    repo_id: HF_E2E_REPOS.vision,
    revision,
    engine_id: "llamacpp",
    task: "vision",
    category: "LLM",
    format_tag: "gguf",
    layout: "gguf_variants",
    artifacts: [
      artifact(
        HF_E2E_ARTIFACTS.visionSharded,
        HF_E2E_DOWNLOAD_IDS.visionSharded,
        "Vision Q4_K_M · 2-part",
        visionShards,
        {
          primaryFile: visionShards[0].path,
          quantType: "Q4_K_M",
        },
      ),
    ],
    companion_artifacts: [
      artifact(
        HF_E2E_ARTIFACTS.projectorF16,
        "hf-e2e-mmproj-f16",
        "Projector F16",
        [
          {
            path: "mmproj-model-f16.gguf",
            size: 900_000_000,
            size_display: "900 MB",
          },
        ],
        { quantType: "F16", isMmproj: true },
      ),
      artifact(
        HF_E2E_ARTIFACTS.projectorQ5,
        "hf-e2e-mmproj-q5-k-m",
        "Projector Q5_K_M",
        [
          {
            path: "mmproj-model-q5_k_m.gguf",
            size: 520_000_000,
            size_display: "520 MB",
          },
        ],
        { quantType: "Q5_K_M", isMmproj: true },
      ),
    ],
    warnings: ["A matching projector is required for image input."],
  },
  [HF_E2E_REPOS.gated]: {
    repo_id: HF_E2E_REPOS.gated,
    revision,
    engine_id: "llamacpp",
    task: "chat",
    category: "LLM",
    format_tag: "gguf",
    layout: "gguf_variants",
    artifacts: [
      artifact(
        HF_E2E_ARTIFACTS.gated,
        HF_E2E_DOWNLOAD_IDS.gated,
        "Gated Q4_K_M",
        [
          {
            path: "gated-chat-q4_k_m.gguf",
            size: 2_700_000_000,
            size_display: "2.7 GB",
          },
        ],
        { quantType: "Q4_K_M" },
      ),
    ],
    companion_artifacts: [],
    warnings: [],
  },
} as const;

const profiles = [
  {
    engine_id: "llamacpp",
    task: "chat",
    category: "LLM",
    pipeline_tags: ["text-generation"],
    format_tag: "gguf",
    layout: "gguf_variants",
    searchable: true,
    compatibility_hint:
      "Only complete GGUF variants supported by the active llama.cpp runtime are shown.",
  },
  {
    engine_id: "llamacpp",
    task: "vision",
    category: "LLM",
    pipeline_tags: ["image-text-to-text"],
    format_tag: "gguf",
    layout: "gguf_variants",
    searchable: true,
    compatibility_hint:
      "Vision installs include a backend-matched multimodal projector.",
  },
  {
    engine_id: "llamacpp",
    task: "embedding",
    category: "Embedding",
    pipeline_tags: ["feature-extraction"],
    format_tag: "gguf",
    layout: "gguf_variants",
    searchable: true,
    compatibility_hint:
      "Only GGUF embedding models accepted by llama.cpp are shown.",
  },
  {
    engine_id: "llamacpp",
    task: "stt",
    category: "STT",
    pipeline_tags: ["automatic-speech-recognition"],
    format_tag: "gguf",
    layout: "gguf_variants",
    searchable: false,
    compatibility_hint: "This deliberately non-searchable profile must stay hidden.",
  },
  {
    engine_id: "mlx",
    task: "diffusion",
    category: "Diffusion",
    pipeline_tags: ["text-to-image"],
    format_tag: "safetensors",
    layout: "directory",
    searchable: true,
    compatibility_hint: "This foreign-engine profile must stay hidden.",
  },
] as const;

const fixtureData = {
  revision,
  repos: HF_E2E_REPOS,
  artifacts: HF_E2E_ARTIFACTS,
  downloads: HF_E2E_DOWNLOAD_IDS,
  cards: {
    chat: modelCard(HF_E2E_REPOS.chat),
    vision: modelCard(HF_E2E_REPOS.vision, { downloads: 8_765, likes: 210 }),
    gated: modelCard(HF_E2E_REPOS.gated, {
      gated: true,
      downloads: 4_321,
      likes: 98,
    }),
    stale: modelCard(HF_E2E_REPOS.stale),
    latest: modelCard(HF_E2E_REPOS.latest),
  },
  plans,
  profiles,
};

/**
 * Injected before the application bundle. Keep this script self-contained:
 * everything it needs is serialized in `fixtureData`.
 */
export function createHfBrowserFixtureScript(): string {
  return `
  (() => {
    const data = ${JSON.stringify(fixtureData)};
    const clone = value => structuredClone(value);
    const state = {
      calls: [],
      inventory: [],
      pendingDownloads: new Map(),
      pendingSearches: new Map(),
      failNextDownload: false,
      privateAccess: false,
    };
    const mocks = window.__wdio_mocks__ ??= {};
    const record = (command, args) => {
      state.calls.push({ command, args: clone(args ?? null) });
    };
    const tracked = (command, implementation) => {
      mocks[command] = async args => {
        record(command, args);
        return implementation(args ?? {});
      };
    };
    const searchResponse = (task, models, hasMore = false) => ({
      engine_id: "llamacpp",
      task,
      models: clone(models),
      has_more: hasMore,
    });
    const modelForSelection = request => {
      const plan = data.plans[request.repo_id];
      const artifact = plan?.artifacts.find(item => item.id === request.artifact_id);
      const companion = plan?.companion_artifacts.find(
        item => item.id === request.companion_artifact_id,
      );
      if (!plan || !artifact) throw new Error("Fixture received an unknown artifact");
      const installRoot = "/tmp/thinclaw-browser-e2e/models/"
        + request.repo_id.replace("/", "--")
        + "/"
        + request.revision
        + "/"
        + request.artifact_id;
      const modelPath = installRoot + "/" + artifact.primary_file;
      return {
        inventory: {
          name: request.repo_id,
          size: artifact.total_size + (companion?.total_size ?? 0),
          path: modelPath,
          id: JSON.stringify([
            request.repo_id,
            request.revision,
            request.artifact_id,
            request.companion_artifact_id,
          ]),
          relative_path: request.repo_id + "/" + artifact.primary_file,
          install_root: installRoot,
          category: plan.category,
          task: plan.task,
          source: "huggingface",
          repo_id: request.repo_id,
          revision: request.revision,
          artifact_id: request.artifact_id,
          companion_artifact_id: request.companion_artifact_id,
          companion_path: companion ? installRoot + "/" + companion.primary_file : null,
          runtime: plan.engine_id,
          format: "GGUF",
          artifact_kind: "gguf",
          compatible: true,
          compatibility_reason: null,
        },
        result: {
          download_id: artifact.download_id,
          repo_id: request.repo_id,
          revision: request.revision,
          engine_id: plan.engine_id,
          task: plan.task,
          category: plan.category,
          artifact_id: request.artifact_id,
          companion_artifact_id: request.companion_artifact_id,
          destination_dir: installRoot,
          model_path: modelPath,
          companion_path: companion ? installRoot + "/" + companion.primary_file : null,
          downloaded_files: [
            ...artifact.files.map(file => file.path),
            ...(companion?.files.map(file => file.path) ?? []),
          ],
          total_bytes: artifact.total_size + (companion?.total_size ?? 0),
        },
      };
    };

    tracked("direct_runtime_get_active_engine_info", () => ({
      id: "llamacpp",
      display_name: "llama.cpp (browser fixture)",
      available: true,
      requires_setup: false,
      description: "Deterministic browser-E2E llama.cpp runtime",
      hf_tag: "gguf",
      single_file_model: true,
    }));
    tracked("direct_runtime_snapshot", () => ({
      kind: "llamacpp",
      displayName: "llama.cpp (browser fixture)",
      readiness: "ready",
      capabilities: ["chat", "vision", "embedding"],
      supportedCapabilities: ["chat", "vision", "embedding"],
      exposurePolicy: "direct_only",
      unavailableReason: null,
    }));
    tracked("direct_runtime_get_hf_capabilities", () => clone(data.profiles));
    tracked("direct_runtime_discover_hf_models_v2", ({ query = "", task = "chat", limit }) => {
      const normalized = query.trim().toLowerCase();
      if (normalized === "slow") {
        return new Promise(resolve => {
          state.pendingSearches.set(normalized, () => resolve(
            searchResponse(task, [data.cards.stale]),
          ));
        });
      }
      if (normalized === "latest") {
        return searchResponse(task, [data.cards.latest]);
      }
      if (normalized.includes("gated") || normalized.includes("private")) {
        return searchResponse(task, [data.cards.gated]);
      }
      if (task === "vision") return searchResponse(task, [data.cards.vision]);
      if (task === "embedding") return searchResponse(task, []);

      const base = [data.cards.chat, data.cards.gated];
      const expanded = Number(limit) > 15
        ? [...base, data.cards.latest]
        : base;
      return searchResponse(task, expanded, Number(limit) <= 15);
    });
    tracked("direct_runtime_get_model_files_v2", ({ repoId }) => {
      if (repoId === data.repos.gated && !state.privateAccess) {
        throw new Error(
          "Hugging Face access denied (HTTP 401). Add a token and accept the repository license.",
        );
      }
      const plan = data.plans[repoId];
      if (!plan) throw new Error("No deterministic plan for " + repoId);
      return clone(plan);
    });
    tracked("direct_runtime_download_hf_selection", ({ request }) => {
      if (state.failNextDownload) {
        state.failNextDownload = false;
        throw new Error("Temporary Hugging Face CDN failure (HTTP 503)");
      }
      const prepared = modelForSelection(request);
      return new Promise((resolve, reject) => {
        state.pendingDownloads.set(prepared.result.download_id, {
          request: clone(request),
          prepared,
          resolve,
          reject,
        });
      });
    });
    tracked("cancel_download", ({ filename }) => {
      const pending = state.pendingDownloads.get(filename);
      if (pending) {
        state.pendingDownloads.delete(filename);
        pending.reject(new Error("Hugging Face download cancelled"));
      }
      return null;
    });
    tracked("list_models", () => clone(state.inventory));
    tracked("open_url", () => null);
    tracked("get_hf_token", () => state.privateAccess ? "hf_e2e_token" : null);
    tracked("thinclaw_set_hf_token", ({ token }) => {
      state.privateAccess = Boolean(token?.trim());
      return null;
    });

    window.__thinclaw_hf_e2e__ = {
      data: clone(data),
      calls(command) {
        return clone(
          command
            ? state.calls.filter(call => call.command === command)
            : state.calls,
        );
      },
      clearCalls() {
        state.calls.length = 0;
      },
      releaseSearch(query) {
        const key = query.trim().toLowerCase();
        const release = state.pendingSearches.get(key);
        if (!release) return false;
        state.pendingSearches.delete(key);
        release();
        return true;
      },
      pendingDownloadIds() {
        return [...state.pendingDownloads.keys()];
      },
      pendingSearchQueries() {
        return [...state.pendingSearches.keys()];
      },
      lastDownloadRequest() {
        const pending = [...state.pendingDownloads.values()].at(-1);
        return pending ? clone(pending.request) : null;
      },
      failNextDownload() {
        state.failNextDownload = true;
      },
      completeDownload(downloadId) {
        const pending = state.pendingDownloads.get(downloadId);
        if (!pending) return false;
        state.pendingDownloads.delete(downloadId);
        state.inventory = [
          ...state.inventory.filter(model => model.id !== pending.prepared.inventory.id),
          pending.prepared.inventory,
        ];
        pending.resolve(clone(pending.prepared.result));
        return true;
      },
      grantPrivateAccess() {
        state.privateAccess = true;
      },
      async emitDownloadProgress(downloadId, percentage, currentFile, fileIndex, fileCount) {
        return window.__TAURI_INTERNALS__.invoke("plugin:event|emit", {
          event: "download_progress",
          payload: {
            filename: downloadId,
            total: 1000,
            downloaded: Math.round(percentage * 10),
            percentage,
            current_file: currentFile,
            file_index: fileIndex,
            file_count: fileCount,
            file_percentage: percentage,
          },
        });
      },
    };
  })();
  `;
}
