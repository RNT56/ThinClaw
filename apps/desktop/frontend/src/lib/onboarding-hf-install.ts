/**
 * Pure, sequential Hugging Face installation orchestration for onboarding.
 *
 * This module deliberately has no React, Tauri, toast, or storage dependency.
 * The wizard supplies validated plans plus its download and path-setting
 * callbacks; failures propagate to the wizard's existing error boundary.
 */
import { requiresHfCompanionArtifact } from "./hf-models";

export type OnboardingHfCategory = "llm" | "embedding" | "stt" | "diffusion";

export type OnboardingHfCategoryEnabled = Record<OnboardingHfCategory, boolean>;

const ONBOARDING_HF_CATEGORIES: readonly OnboardingHfCategory[] = [
    "llm",
    "embedding",
    "stt",
    "diffusion",
];

const DEFAULT_CATEGORY_ENABLED: OnboardingHfCategoryEnabled = {
    llm: true,
    embedding: true,
    stt: false,
    diffusion: false,
};

/**
 * Reconcile category toggles only from an authoritative capability response.
 *
 * Existing choices for still-available optional categories are preserved.
 * Newly available categories receive their onboarding defaults, unavailable
 * categories are disabled, and the required LLM category is always enabled
 * whenever the runtime exposes it.
 */
export function reconcileOnboardingHfCategoryEnabled({
    previous,
    availableCategories,
    previousAvailableCategories,
    authoritative,
    resetDefaults = false,
}: {
    previous: OnboardingHfCategoryEnabled;
    availableCategories: readonly OnboardingHfCategory[];
    previousAvailableCategories: readonly OnboardingHfCategory[];
    authoritative: boolean;
    resetDefaults?: boolean;
}): OnboardingHfCategoryEnabled {
    if (!authoritative) return previous;

    const available = new Set(availableCategories);
    const previouslyAvailable = resetDefaults
        ? new Set<OnboardingHfCategory>()
        : new Set(previousAvailableCategories);
    const next = { ...previous };

    for (const category of ONBOARDING_HF_CATEGORIES) {
        if (!available.has(category)) {
            next[category] = false;
        } else if (category === "llm") {
            next.llm = true;
        } else if (!previouslyAvailable.has(category)) {
            next[category] = DEFAULT_CATEGORY_ENABLED[category];
        }
    }

    return ONBOARDING_HF_CATEGORIES.every(
        category => next[category] === previous[category],
    )
        ? previous
        : next;
}

export interface OnboardingHfArtifactLike {
    id: string;
    download_id: string;
}

export interface OnboardingHfPlanLike<
    TTask extends string = string,
    TArtifact extends OnboardingHfArtifactLike = OnboardingHfArtifactLike,
> {
    repo_id: string;
    revision: string;
    engine_id: string;
    task: TTask;
    category: string;
    artifacts: readonly TArtifact[];
    companion_artifacts: readonly TArtifact[];
}

export interface OnboardingHfInstallSelection<
    TTask extends string = string,
    TArtifact extends OnboardingHfArtifactLike = OnboardingHfArtifactLike,
> {
    category: OnboardingHfCategory;
    plan: OnboardingHfPlanLike<TTask, TArtifact>;
    artifact: TArtifact;
    companion?: TArtifact | null;
    existingModelPath?: string | null;
    destinationName?: string | null;
}

/** Structurally matches the generated `HfDownloadSelectionRequest`. */
export interface OnboardingHfDownloadRequest<TTask extends string = string> {
    repo_id: string;
    revision: string;
    task: TTask;
    artifact_id: string;
    companion_artifact_id: string | null;
    destination_name: string | null;
}

/** Required result fields used to verify provenance and select the runtime path. */
export interface OnboardingHfDownloadResult<TTask extends string = string> {
    download_id: string;
    repo_id: string;
    revision: string;
    engine_id: string;
    task: TTask;
    category: string;
    artifact_id: string;
    companion_artifact_id: string | null;
    model_path: string;
}

export interface OnboardingHfInstallOutcome<
    TTask extends string = string,
    TResult extends OnboardingHfDownloadResult<TTask> =
        OnboardingHfDownloadResult<TTask>,
> {
    category: OnboardingHfCategory;
    modelPath: string;
    source: "installed" | "downloaded";
    downloadResult: TResult | null;
}

export interface InstallOnboardingHfSelectionsOptions<
    TTask extends string = string,
    TArtifact extends OnboardingHfArtifactLike = OnboardingHfArtifactLike,
    TResult extends OnboardingHfDownloadResult<TTask> =
        OnboardingHfDownloadResult<TTask>,
> {
    selections: readonly OnboardingHfInstallSelection<TTask, TArtifact>[];
    download: (
        request: OnboardingHfDownloadRequest<TTask>,
        downloadId: string,
    ) => Promise<TResult>;
    setPath: (
        category: OnboardingHfCategory,
        absoluteModelPath: string,
    ) => void | Promise<void>;
}

const PLAN_CATEGORY: Record<OnboardingHfCategory, string> = {
    llm: "LLM",
    embedding: "Embedding",
    stt: "STT",
    diffusion: "Diffusion",
};

/** Browser-safe absolute-path check covering Unix, Windows drive, and UNC paths. */
export function isAbsoluteModelPath(path: string): boolean {
    return path.startsWith("/")
        || /^[A-Za-z]:[\\/]/.test(path)
        || /^\\\\[^\\]+\\[^\\]+/.test(path);
}

function validateSelection<
    TTask extends string,
    TArtifact extends OnboardingHfArtifactLike,
>(selection: OnboardingHfInstallSelection<TTask, TArtifact>): void {
    const { artifact, category, companion, existingModelPath, plan } = selection;
    if (!plan.repo_id || !plan.revision || !plan.engine_id || !plan.task) {
        throw new Error(`Incomplete Hugging Face plan for ${category}`);
    }
    if (plan.category !== PLAN_CATEGORY[category]) {
        throw new Error(
            `Hugging Face plan category ${plan.category} does not match ${category}`,
        );
    }
    const plannedArtifact = plan.artifacts.find(
        (candidate) => candidate.id === artifact.id,
    );
    if (!plannedArtifact || plannedArtifact.download_id !== artifact.download_id) {
        throw new Error(
            `Selected Hugging Face artifact is not in the pinned plan for ${plan.repo_id}`,
        );
    }
    if (companion) {
        const plannedCompanion = plan.companion_artifacts.find(
            (candidate) => candidate.id === companion.id,
        );
        if (
            !plannedCompanion
            || plannedCompanion.download_id !== companion.download_id
        ) {
            throw new Error(
                `Selected Hugging Face companion is not in the pinned plan for ${plan.repo_id}`,
            );
        }
    }
    if (requiresHfCompanionArtifact(plan) && !companion) {
        throw new Error(
            `A vision projector is required for ${plan.repo_id} with llama.cpp`,
        );
    }
    if (existingModelPath && !isAbsoluteModelPath(existingModelPath)) {
        throw new Error(`Installed model path for ${plan.repo_id} is not absolute`);
    }
}

function verifyDownloadResult<TTask extends string>(
    selection: OnboardingHfInstallSelection<TTask>,
    result: OnboardingHfDownloadResult<TTask>,
): void {
    const { artifact, plan } = selection;
    if (
        result.download_id !== artifact.download_id
        || result.repo_id !== plan.repo_id
        || result.revision !== plan.revision
        || result.engine_id !== plan.engine_id
        || result.task !== plan.task
        || result.category !== plan.category
        || result.artifact_id !== artifact.id
        || result.companion_artifact_id !== (selection.companion?.id ?? null)
    ) {
        throw new Error(
            `Hugging Face download result did not match the pinned plan for ${plan.repo_id}`,
        );
    }
    if (!isAbsoluteModelPath(result.model_path)) {
        throw new Error(
            `Hugging Face download returned a non-absolute model path for ${plan.repo_id}`,
        );
    }
}

/**
 * Install onboarding selections one at a time.
 *
 * Sequential awaiting intentionally avoids several large downloads competing
 * for disk and network resources. The path setter runs only after a verified
 * download result, or with a verified absolute path for an existing install.
 * Any failure rejects immediately and prevents all later selections and path
 * setters from running.
 */
export async function installOnboardingHfSelections<
    TTask extends string,
    TArtifact extends OnboardingHfArtifactLike,
    TResult extends OnboardingHfDownloadResult<TTask>,
>({
    selections,
    download,
    setPath,
}: InstallOnboardingHfSelectionsOptions<
    TTask,
    TArtifact,
    TResult
>): Promise<OnboardingHfInstallOutcome<TTask, TResult>[]> {
    const categories = new Set<OnboardingHfCategory>();
    for (const selection of selections) {
        if (categories.has(selection.category)) {
            throw new Error(
                `Duplicate Hugging Face onboarding selection for ${selection.category}`,
            );
        }
        categories.add(selection.category);
        validateSelection(selection);
    }

    const outcomes: OnboardingHfInstallOutcome<TTask, TResult>[] = [];
    for (const selection of selections) {
        const existingModelPath = selection.existingModelPath ?? null;
        if (existingModelPath) {
            await setPath(selection.category, existingModelPath);
            outcomes.push({
                category: selection.category,
                modelPath: existingModelPath,
                source: "installed",
                downloadResult: null,
            });
            continue;
        }

        const request: OnboardingHfDownloadRequest<TTask> = {
            repo_id: selection.plan.repo_id,
            revision: selection.plan.revision,
            task: selection.plan.task,
            artifact_id: selection.artifact.id,
            companion_artifact_id: selection.companion?.id ?? null,
            destination_name: selection.destinationName ?? null,
        };
        const result = await download(request, selection.artifact.download_id);
        verifyDownloadResult(selection, result);
        await setPath(selection.category, result.model_path);
        outcomes.push({
            category: selection.category,
            modelPath: result.model_path,
            source: "downloaded",
            downloadResult: result,
        });
    }
    return outcomes;
}
