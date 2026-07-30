export function shouldIncludeCuratedEntryInMyModels(
    model: { category?: string | null },
): boolean {
    // Downloaded local models come exclusively from backend inventory. Curated
    // local entries belong in Discover; configured cloud catalog entries remain
    // useful in My Models because they have no on-disk inventory record.
    return model.category === "Cloud";
}

export function isModelOnDiskInLibrary(
    model: { isLocal?: boolean | null },
): boolean {
    return model.isLocal === true;
}

export function buildModelLibraryCategories({
    hasAnyCloud,
    isLlamaCpp,
    summarizerSupported,
    supportedCapabilities,
}: {
    hasAnyCloud: boolean;
    isLlamaCpp: boolean;
    summarizerSupported: boolean;
    supportedCapabilities: readonly string[];
}): string[] {
    const supported = new Set(supportedCapabilities);
    const categories = ["All", ...(hasAnyCloud ? ["Cloud Brains"] : []), "Chat"];
    if (summarizerSupported) categories.push("Summarizer");
    if (supported.has("diffusion")) categories.push("Diffusion");
    if (supported.has("stt")) categories.push("STT");
    if (supported.has("embedding")) categories.push("Embedding");
    if (supported.has("tts")) categories.push("TTS");
    if (isLlamaCpp) categories.push("Standard");
    return categories;
}

export function normalizeModelLibraryCategory(
    activeCategory: string,
    availableCategories: readonly string[],
): string {
    return availableCategories.includes(activeCategory) ? activeCategory : "All";
}

export interface SummarizerEngineState {
    id: string;
    available: boolean;
    single_file_model: boolean;
}

export interface SummarizerRuntimeState {
    kind: string;
    supportedCapabilities?: readonly string[] | null;
}

export interface SummarizerCandidate {
    isLocal?: boolean | null;
    localPath?: string | null;
    managedCategory?: string | null;
    compatible?: boolean | null;
}

/**
 * The dedicated summarizer server is a llama.cpp GGUF sidecar. EngineInfo
 * identifies the compiled backend while LocalRuntimeSnapshot confirms the
 * runtime family and its chat capability; requiring both prevents stale UI
 * state from exposing the control after an engine switch.
 */
export function supportsLocalSummarizer(
    engine: SummarizerEngineState | null,
    runtime: SummarizerRuntimeState | null,
): boolean {
    return Boolean(
        engine?.available
        && engine.id === "llamacpp"
        && engine.single_file_model
        && runtime?.kind === "llama_cpp"
        && runtime.supportedCapabilities?.includes("chat")
    );
}

/**
 * Summarizer starts are inventory-authoritative in the backend. Only a
 * compatible, installed LLM entry with an exact managed path can be offered.
 */
export function isLocalSummarizerCandidate(
    model: SummarizerCandidate,
    runtimeSupported: boolean,
): boolean {
    return Boolean(
        runtimeSupported
        && model.isLocal
        && model.compatible === true
        && model.managedCategory === "LLM"
        && model.localPath
    );
}

export interface VisionSelectionState {
    selected: boolean;
    operational: boolean;
}

/**
 * A stored vision preference can be cleared even when it is stale. It becomes
 * operational only when the same model is also the active chat model.
 */
export function getVisionSelectionState(
    modelPath: string | null | undefined,
    currentChatModelPath: string,
    currentVisionModelPath: string,
): VisionSelectionState {
    const selected = Boolean(
        modelPath
        && currentVisionModelPath
        && modelPath === currentVisionModelPath
    );
    return {
        selected,
        operational: selected && modelPath === currentChatModelPath,
    };
}

export interface ModelDeactivationRoles {
    chat: boolean;
    embedding: boolean;
    vision: boolean;
    summarizer: boolean;
    stt: boolean;
    image: boolean;
}

export interface ModelDeactivationPlan {
    hasSelection: boolean;
    stopEngine: boolean;
    deactivateServices: boolean;
    services: Omit<ModelDeactivationRoles, "vision">;
    clearSelections: ModelDeactivationRoles;
}

/**
 * Build the smallest deactivation operation for one inventory entry. Vision
 * shares the chat runtime, so a stale vision-only preference is cleared without
 * stopping any service. Directory-backed chat engines need their engine process
 * stopped in addition to the role-specific sidecar cleanup.
 */
export function buildModelDeactivationPlan(
    roles: ModelDeactivationRoles,
    engine: {
        id: string;
        single_file_model: boolean;
    } | null,
): ModelDeactivationPlan {
    const services = {
        chat: roles.chat,
        embedding: roles.embedding,
        summarizer: roles.summarizer,
        stt: roles.stt,
        image: roles.image,
    };
    return {
        hasSelection: Object.values(roles).some(Boolean),
        stopEngine: Boolean(
            roles.chat
            && engine
            && !engine.single_file_model
            && engine.id !== "none"
            && engine.id !== "ollama"
        ),
        deactivateServices: Object.values(services).some(Boolean),
        services,
        clearSelections: { ...roles },
    };
}

export interface RemovableModelIdentity {
    path?: string | null;
    companion_path?: string | null;
    relative_path?: string | null;
    install_root?: string | null;
}

export interface ModelSelectionSnapshot {
    chat: string;
    embedding: string;
    vision: string;
    stt: string;
    diffusion: string;
    summarizer: string;
}

const MODEL_SELECTION_ROLES: ReadonlyArray<
    readonly [keyof ModelSelectionSnapshot, string]
> = [
    ["chat", "chat"],
    ["embedding", "embedding"],
    ["vision", "vision"],
    ["stt", "speech-to-text"],
    ["diffusion", "image generation"],
    ["summarizer", "summarizer"],
];

export function modelRemovalPaths(
    model: RemovableModelIdentity | undefined,
): Set<string> {
    return new Set([
        model?.path,
        model?.companion_path,
        model?.relative_path,
        model?.install_root,
    ].filter((path): path is string => Boolean(path)));
}

function normalizeModelPathForRemoval(path: string): string {
    const normalizedSeparators = path.replace(/\\/g, "/").replace(/\/+/g, "/");
    return normalizedSeparators.length > 1
        ? normalizedSeparators.replace(/\/+$/, "")
        : normalizedSeparators;
}

/**
 * Match both exact artifact paths and children of a managed install root.
 * The explicit separator boundary prevents an install such as `model` from
 * affecting a sibling such as `model-backup`.
 */
export function isModelPathAffectedByRemoval(
    path: string,
    removedPaths: ReadonlySet<string>,
): boolean {
    if (!path) return false;
    const normalizedPath = normalizeModelPathForRemoval(path);
    return Array.from(removedPaths).some((removedPath) => {
        if (!removedPath) return false;
        const normalizedRemovedPath = normalizeModelPathForRemoval(removedPath);
        return normalizedPath === normalizedRemovedPath
            || normalizedPath.startsWith(`${normalizedRemovedPath}/`);
    });
}

export function selectedRolesForModelRemoval(
    removedPaths: ReadonlySet<string>,
    selections: ModelSelectionSnapshot,
): string[] {
    return MODEL_SELECTION_ROLES
        .filter(([key]) => isModelPathAffectedByRemoval(selections[key], removedPaths))
        .map(([, label]) => label);
}

export function selectedModelRolesForRemoval(
    removedPaths: ReadonlySet<string>,
    selections: ModelSelectionSnapshot,
): ModelDeactivationRoles {
    return {
        chat: isModelPathAffectedByRemoval(selections.chat, removedPaths),
        embedding: isModelPathAffectedByRemoval(selections.embedding, removedPaths),
        vision: isModelPathAffectedByRemoval(selections.vision, removedPaths),
        stt: isModelPathAffectedByRemoval(selections.stt, removedPaths),
        image: isModelPathAffectedByRemoval(selections.diffusion, removedPaths),
        summarizer: isModelPathAffectedByRemoval(selections.summarizer, removedPaths),
    };
}
