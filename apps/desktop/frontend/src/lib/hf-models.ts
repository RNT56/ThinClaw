/**
 * Pure Hugging Face discovery helpers shared by settings and onboarding.
 *
 * The structural input types intentionally mirror the generated Rust bindings
 * without importing them. This keeps the helpers usable while bindings are
 * regenerated and makes them straightforward to test with small fixtures.
 */

export const HF_MODEL_TASKS = [
    "chat",
    "vision",
    "embedding",
    "stt",
    "diffusion",
    "tts",
] as const;

export type HfModelTaskId = (typeof HF_MODEL_TASKS)[number];
export type HfModelCategory = "LLM" | "Embedding" | "STT" | "Diffusion" | "TTS";
export type HfRequestStatus = "idle" | "loading" | "ready" | "error";

export interface HfCapabilityProfileLike {
    engine_id: string;
    task: string;
    category: string;
    pipeline_tags: readonly string[];
    format_tag: string;
    searchable: boolean;
    compatibility_hint?: string | null;
}

export interface HfFilterMetadata {
    id: HfModelTaskId;
    task: HfModelTaskId;
    label: string;
    placeholder: string;
    category: HfModelCategory;
    pipelineTags: string[];
    formatTag: string;
    compatibilityHint: string | null;
}

export interface HfArtifactFileLike {
    path: string;
}

export interface HfDownloadArtifactLike {
    id: string;
    label: string;
    files: readonly HfArtifactFileLike[];
    quant_type?: string | null;
    is_mmproj?: boolean;
    total_size?: number;
    /** Forward-compatible hint for a future backend-selected recommendation. */
    recommended?: boolean;
}

export interface HfModelFilePlanLike<
    TArtifact extends HfDownloadArtifactLike = HfDownloadArtifactLike,
> {
    engine_id?: string;
    task?: string;
    artifacts: readonly TArtifact[];
    companion_artifacts?: readonly TArtifact[];
}

export interface InstalledModelIdentityLike {
    repo_id?: string | null;
    revision?: string | null;
    artifact_id?: string | null;
    companion_artifact_id?: string | null;
    runtime?: string | null;
    task?: string | null;
    compatible?: boolean;
}

export interface HfDownloadSelectionIdentityLike {
    repo_id: string;
    revision: string;
    task: string;
    artifact_id: string;
    companion_artifact_id?: string | null;
    destination_name?: string | null;
}

export interface ManagedModelCategoryLike {
    category: string;
    compatible: boolean;
}

export interface HfArtifactSelection<
    TArtifact extends HfDownloadArtifactLike = HfDownloadArtifactLike,
> {
    artifact: TArtifact;
    companion: TArtifact | null;
    filePaths: string[];
}

/**
 * llama.cpp vision inference is only operational when the selected model is
 * installed together with a matching multimodal projector. Other runtimes
 * either package vision assets in the model directory or do not expose the
 * vision profile.
 */
export function requiresHfCompanionArtifact(
    plan: Pick<HfModelFilePlanLike, "engine_id" | "task">,
): boolean {
    return plan.engine_id === "llamacpp" && plan.task === "vision";
}

/**
 * Resolve the effective companion selection. Required projector plans default
 * to their first backend-ranked companion; optional plans preserve "none".
 */
export function effectiveHfCompanionArtifactId<
    TArtifact extends HfDownloadArtifactLike,
>(
    plan: HfModelFilePlanLike<TArtifact>,
    selectedId?: string | null,
): string | null {
    if (
        selectedId
        && plan.companion_artifacts?.some(artifact => artifact.id === selectedId)
    ) {
        return selectedId;
    }
    if (!requiresHfCompanionArtifact(plan)) return null;
    return plan.companion_artifacts?.[0]?.id ?? null;
}

interface TaskUiDefinition {
    label: string;
    placeholder: string;
    category: HfModelCategory;
}

const TASK_UI: Record<HfModelTaskId, TaskUiDefinition> = {
    chat: {
        label: "Text",
        placeholder: "Search chat models…",
        category: "LLM",
    },
    vision: {
        label: "Vision",
        placeholder: "Search vision models…",
        category: "LLM",
    },
    embedding: {
        label: "Embedding",
        placeholder: "Search embedding models…",
        category: "Embedding",
    },
    stt: {
        label: "Speech-to-Text",
        placeholder: "Search speech recognition models…",
        category: "STT",
    },
    diffusion: {
        label: "Image Generation",
        placeholder: "Search image generation models…",
        category: "Diffusion",
    },
    tts: {
        label: "Text-to-Speech",
        placeholder: "Search speech synthesis models…",
        category: "TTS",
    },
};

const CATEGORY_TASKS: Record<HfModelCategory, readonly HfModelTaskId[]> = {
    LLM: ["chat", "vision"],
    Embedding: ["embedding"],
    STT: ["stt"],
    Diffusion: ["diffusion"],
    TTS: ["tts"],
};

const QUANTIZATION_PREFERENCE = [
    "Q4_K_M",
    "Q5_K_M",
    "Q4_K_S",
    "Q5_K_S",
    "Q6_K",
    "Q8_0",
    "IQ4_XS",
    "IQ4_NL",
    "Q3_K_M",
    "Q3_K_S",
    "Q2_K",
    "BF16",
    "F16",
    "F32",
] as const;

function isHfModelTask(value: string): value is HfModelTaskId {
    return (HF_MODEL_TASKS as readonly string[]).includes(value);
}

function normalizeCategory(value: string): HfModelCategory | null {
    const normalized = value.trim().toLowerCase();
    if (normalized === "llm" || normalized === "chat" || normalized === "vision") {
        return "LLM";
    }
    if (normalized === "embedding") return "Embedding";
    if (normalized === "stt") return "STT";
    if (normalized === "diffusion") return "Diffusion";
    if (normalized === "tts") return "TTS";
    return null;
}

/** Collision-safe identity for download and installed-artifact maps. */
export function artifactKey(repoId: string, artifactId: string): string {
    if (!repoId || !artifactId) {
        throw new Error("Hugging Face repository and artifact IDs must be non-empty");
    }
    return JSON.stringify([repoId, artifactId]);
}

/** Canonical managed-model category for a discovery task. */
export function categoryForTask(task: HfModelTaskId): HfModelCategory {
    return TASK_UI[task].category;
}

/** Every discovery task that can live in a managed-model category. */
export function tasksForCategory(category: string): readonly HfModelTaskId[] {
    const normalized = normalizeCategory(category);
    return normalized ? CATEGORY_TASKS[normalized] : [];
}

/**
 * Convert one backend capability profile to display metadata.
 * Non-searchable and unknown future tasks are deliberately omitted.
 */
export function profileToFilterMetadata(
    profile: HfCapabilityProfileLike,
): HfFilterMetadata | null {
    if (!profile.searchable || !isHfModelTask(profile.task)) return null;

    const definition = TASK_UI[profile.task];
    const category = normalizeCategory(profile.category) ?? definition.category;
    return {
        id: profile.task,
        task: profile.task,
        label: definition.label,
        placeholder: definition.placeholder,
        category,
        pipelineTags: [...profile.pipeline_tags],
        formatTag: profile.format_tag,
        compatibilityHint: profile.compatibility_hint ?? null,
    };
}

/**
 * Build a stable, deduplicated filter list from the backend-owned profiles.
 * Passing an engine ID protects consumers that receive profiles for all builds.
 */
export function filtersFromProfiles(
    profiles: readonly HfCapabilityProfileLike[],
    engineId?: string,
): HfFilterMetadata[] {
    const byTask = new Map<HfModelTaskId, HfFilterMetadata>();
    for (const profile of profiles) {
        if (engineId !== undefined && profile.engine_id !== engineId) continue;
        const filter = profileToFilterMetadata(profile);
        if (filter && !byTask.has(filter.task)) byTask.set(filter.task, filter);
    }
    return HF_MODEL_TASKS.flatMap((task) => {
        const filter = byTask.get(task);
        return filter ? [filter] : [];
    });
}

function normalizedQuantization(artifact: HfDownloadArtifactLike): string {
    return (artifact.quant_type ?? artifact.label)
        .trim()
        .toUpperCase()
        .replace(/-/g, "_");
}

function quantizationRank(artifact: HfDownloadArtifactLike): number {
    const quantization = normalizedQuantization(artifact);
    const rank = QUANTIZATION_PREFERENCE.findIndex((candidate) =>
        quantization.includes(candidate),
    );
    return rank === -1 ? Number.MAX_SAFE_INTEGER : rank;
}

/**
 * Choose one complete artifact, never a combination of alternative artifacts.
 * An explicit backend recommendation wins; otherwise Q4_K_M is preferred as a
 * balanced GGUF default. Backend order is retained as the final tie-breaker.
 */
export function selectRecommendedArtifact<
    TArtifact extends HfDownloadArtifactLike,
>(artifacts: readonly TArtifact[]): TArtifact | null {
    const candidates = artifacts.filter((artifact) => !artifact.is_mmproj);
    if (candidates.length === 0) return null;

    const explicit = candidates.find((artifact) => artifact.recommended);
    if (explicit) return explicit;

    let best = candidates[0];
    let bestRank = quantizationRank(best);
    for (const candidate of candidates.slice(1)) {
        const rank = quantizationRank(candidate);
        if (rank < bestRank) {
            best = candidate;
            bestRank = rank;
        }
    }
    return best;
}

/** Required file paths for exactly one artifact and optional companion. */
export function artifactFilePaths(
    artifact: HfDownloadArtifactLike,
    companion?: HfDownloadArtifactLike | null,
): string[] {
    const paths: string[] = [];
    const seen = new Set<string>();
    for (const file of [...artifact.files, ...(companion?.files ?? [])]) {
        if (file.path && !seen.has(file.path)) {
            seen.add(file.path);
            paths.push(file.path);
        }
    }
    return paths;
}

/**
 * Resolve a selected artifact ID to its complete file group. Alternative
 * artifacts are not included; every shard within the selected group is.
 */
export function resolveArtifactSelection<
    TArtifact extends HfDownloadArtifactLike,
>(
    plan: HfModelFilePlanLike<TArtifact>,
    artifactId: string,
    companionArtifactId?: string | null,
): HfArtifactSelection<TArtifact> | null {
    const artifact = plan.artifacts.find((candidate) => candidate.id === artifactId);
    if (!artifact) return null;

    const companion = companionArtifactId
        ? plan.companion_artifacts?.find(
            (candidate) => candidate.id === companionArtifactId,
        ) ?? null
        : null;
    if (companionArtifactId && !companion) return null;

    return {
        artifact,
        companion,
        filePaths: artifactFilePaths(artifact, companion),
    };
}

/** Exact repository provenance match for repository-level badges. */
export function isRepositoryInstalled(
    models: readonly InstalledModelIdentityLike[],
    repoId: string,
): boolean {
    return models.some((model) => model.repo_id === repoId);
}

/** Stable identity for coalescing only byte-for-byte equivalent selections. */
export function hfDownloadSelectionFingerprint(
    selection: HfDownloadSelectionIdentityLike,
): string {
    return JSON.stringify([
        selection.repo_id,
        selection.revision,
        selection.task,
        selection.artifact_id,
        selection.companion_artifact_id ?? null,
        selection.destination_name ?? null,
    ]);
}

/** Only an explicit retry may move a failed top-model request back to idle. */
export function shouldStartHfTopModelsRequest(
    status?: HfRequestStatus,
): boolean {
    return (status ?? "idle") === "idle";
}

export interface InstalledArtifactSelection {
    repoId: string;
    revision: string;
    engineId: string;
    task: HfModelTaskId;
    artifactId: string;
    companionArtifactId?: string | null;
}

/**
 * Find an install that can satisfy the exact pinned selection. A vision
 * install can also serve text chat, but a chat-only install never satisfies a
 * vision selection. When no companion is requested, an install with an extra
 * projector remains a valid superset.
 */
export function findInstalledArtifactSelection<
    TModel extends InstalledModelIdentityLike,
>(
    models: readonly TModel[],
    selection: InstalledArtifactSelection,
): TModel | undefined {
    return models.find(model => {
        const taskMatches = model.task === selection.task
            || (selection.task === "chat" && model.task === "vision");
        const companionMatches = !selection.companionArtifactId
            || model.companion_artifact_id === selection.companionArtifactId;
        return model.compatible !== false
            && model.repo_id === selection.repoId
            && model.revision === selection.revision
            && model.runtime === selection.engineId
            && taskMatches
            && model.artifact_id === selection.artifactId
            && companionMatches;
    });
}

/** Category/runtime filtering for local selectors; filenames are irrelevant. */
export function isCompatibleManagedModelForCategory(
    model: ManagedModelCategoryLike,
    category: HfModelCategory,
): boolean {
    return model.compatible && normalizeCategory(model.category) === category;
}

/** Resolve a preferred compatible inventory entry, then the first fallback. */
export function resolveCompatibleManagedModel<
    TModel extends ManagedModelCategoryLike & { path: string },
>(
    models: readonly TModel[],
    category: HfModelCategory,
    preferredPath?: string | null,
): TModel | undefined {
    const isMatch = (model: TModel) =>
        isCompatibleManagedModelForCategory(model, category);
    return (
        (preferredPath
            ? models.find(model => model.path === preferredPath && isMatch(model))
            : undefined)
        ?? models.find(isMatch)
    );
}

export interface RequestGenerationGuard {
    begin: () => number;
    isCurrent: (generation: number) => boolean;
    invalidate: () => void;
}

/** Small guard for ignoring out-of-order async search responses. */
export function createRequestGenerationGuard(): RequestGenerationGuard {
    let current = 0;
    return {
        begin: () => {
            current += 1;
            return current;
        },
        isCurrent: (generation) => generation === current,
        invalidate: () => {
            current += 1;
        },
    };
}

export interface HfModelCardIdentityLike {
    id: string;
}

/**
 * Merge a larger "first N" Hub response into the cards already on screen.
 * Existing positions stay stable, refreshed metadata wins, and duplicate
 * repository IDs can never produce duplicate cards.
 */
export function mergeHfModelCards<T extends HfModelCardIdentityLike>(
    existing: readonly T[],
    incoming: readonly T[],
): T[] {
    const latestById = new Map(incoming.map(model => [model.id, model]));
    const merged: T[] = [];
    const seen = new Set<string>();

    for (const model of existing) {
        if (!model.id || seen.has(model.id)) continue;
        merged.push(latestById.get(model.id) ?? model);
        seen.add(model.id);
    }
    for (const model of incoming) {
        if (!model.id || seen.has(model.id)) continue;
        merged.push(model);
        seen.add(model.id);
    }
    return merged;
}

export interface HfSearchCacheValue<T> {
    models: readonly T[];
    hasMore: boolean;
    requestedLimit: number;
}

export interface HfSearchCache<T> {
    get: (key: string, now?: number) => HfSearchCacheValue<T> | undefined;
    set: (key: string, value: HfSearchCacheValue<T>, now?: number) => void;
    clear: () => void;
}

/**
 * Small in-memory LRU/TTL cache for per-engine, per-task trending results.
 * It survives discovery component remounts, but never persists Hub responses
 * or credentials to disk.
 */
export function createHfSearchCache<T>(
    ttlMs: number,
    maxEntries = 16,
): HfSearchCache<T> {
    if (!Number.isFinite(ttlMs) || ttlMs <= 0) {
        throw new Error("Hugging Face cache TTL must be positive");
    }
    if (!Number.isInteger(maxEntries) || maxEntries <= 0) {
        throw new Error("Hugging Face cache capacity must be a positive integer");
    }

    const entries = new Map<string, {
        value: HfSearchCacheValue<T>;
        expiresAt: number;
    }>();

    return {
        get: (key, now = Date.now()) => {
            const entry = entries.get(key);
            if (!entry) return undefined;
            if (entry.expiresAt <= now) {
                entries.delete(key);
                return undefined;
            }
            entries.delete(key);
            entries.set(key, entry);
            return entry.value;
        },
        set: (key, value, now = Date.now()) => {
            entries.delete(key);
            entries.set(key, {
                value,
                expiresAt: now + ttlMs,
            });
            while (entries.size > maxEntries) {
                const oldestKey = entries.keys().next().value;
                if (oldestKey === undefined) break;
                entries.delete(oldestKey);
            }
        },
        clear: () => entries.clear(),
    };
}

export type HfHubRemediationKind = "access" | "rate-limit";

/**
 * Classify only errors with strong Hub access/rate-limit signals. Unknown
 * failures retain their original message without speculative advice.
 */
export function classifyHfHubError(
    error: string,
): HfHubRemediationKind | null {
    const normalized = error.toLowerCase();
    if (
        /(?:^|\D)429(?:\D|$)/.test(normalized)
        || normalized.includes("too many requests")
        || normalized.includes("rate limit")
        || normalized.includes("rate-limit")
    ) {
        return "rate-limit";
    }
    if (
        /(?:^|\D)(?:401|403)(?:\D|$)/.test(normalized)
        || normalized.includes("unauthorized")
        || normalized.includes("forbidden")
        || normalized.includes("gated repo")
        || normalized.includes("gated model")
        || normalized.includes("private repo")
        || normalized.includes("private model")
        || normalized.includes("access to this model")
        || normalized.includes("huggingface token")
        || normalized.includes("hugging face token")
    ) {
        return "access";
    }
    return null;
}

/** Build a canonical Hub model URL without allowing query/fragment injection. */
export function huggingFaceRepositoryUrl(repoId: string): string | null {
    const segments = repoId.trim().split("/");
    if (
        segments.length !== 2
        || segments.some(segment =>
            !segment
            || segment === "."
            || segment === ".."
            || segment.length > 128
            || /[\u0000-\u001f\u007f]/.test(segment)
        )
    ) {
        return null;
    }
    return `https://huggingface.co/${segments.map(encodeURIComponent).join("/")}`;
}
