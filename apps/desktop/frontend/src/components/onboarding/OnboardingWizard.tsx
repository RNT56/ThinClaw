import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    CheckCircle, ChevronRight, Monitor, Globe, Cpu, Code, HardDrive, Info, Palette, Moon, Sun,
    Search, Heart, ArrowDownToLine, Loader2, Zap, Wrench, AlertTriangle, CheckCircle2, RefreshCw,
    Server, Key, Bot, Database, Image, Mic, Type
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { useTheme } from '../theme-provider';
import { APP_THEMES } from '../../lib/app-themes';
import { toast } from 'sonner';
// model-library no longer used directly — all models discovered via HF Hub
import { useModelContext } from '../model-context';
import {
    commands,
    type HfCapabilityProfileDto,
    type HfModelCard,
    type HfModelFilePlan,
    type HfModelTask,
} from '../../lib/bindings';
import { listen } from '@tauri-apps/api/event';
import { useEngineSetup } from '../../hooks/use-engine-setup';
import { clearOnboardingProgress, startOnboardingProgress } from '../../lib/local-storage-migration';
import { directCommands } from '../../lib/generated/direct-commands';
import { unwrapResult } from '../../lib/guards';
import { bridgeErrorMessage } from '../../lib/command-errors';
import { Progress } from '../ui';
import {
    createRequestGenerationGuard,
    effectiveHfCompanionArtifactId,
    findInstalledArtifactSelection,
    requiresHfCompanionArtifact,
    shouldStartHfTopModelsRequest,
    selectRecommendedArtifact,
    type HfModelFilePlanLike,
} from '../../lib/hf-models';
import {
    installOnboardingHfSelections,
    reconcileOnboardingHfCategoryEnabled,
    type OnboardingHfCategory,
} from '../../lib/onboarding-hf-install';
import { useOllamaModels, type OllamaModelsStatus } from '../../hooks/use-ollama-models';
import {
    chooseInstalledOllamaModel,
    isInstalledOllamaModel,
} from '../../lib/ollama-models';

function formatDownloads(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toString();
}

interface OnboardingWizardProps {
    onComplete: () => void;
}

export type Step = 'welcome' | 'style' | 'mode' | 'remote_setup' | 'agent' | 'engine_setup' | 'inference' | 'models' | 'api_keys' | 'permissions' | 'complete';
export type InferenceChoice = 'local' | 'cloud';
export type OnboardingLocalInferenceSupport =
    | 'checking'
    | 'available'
    | 'externally_managed'
    | 'unavailable'
    | 'error';
type ModelCategory = OnboardingHfCategory;
const hfPlanKey = (category: ModelCategory, repoId: string) => `${category}:${repoId}`;

export function resolveOnboardingLocalInferenceSupport({
    engineId,
    profiles,
    capabilitiesLoading,
    capabilitiesError,
}: {
    engineId: string | null | undefined;
    profiles: readonly Pick<HfCapabilityProfileDto, 'engine_id' | 'searchable'>[];
    capabilitiesLoading: boolean;
    capabilitiesError: string | null;
}): OnboardingLocalInferenceSupport {
    if (!engineId || capabilitiesLoading) return 'checking';

    // Ollama deliberately owns model acquisition outside the Hugging Face
    // workflow, while `none` deliberately means this build has no local
    // inference runtime at all.
    if (engineId === 'ollama') return 'externally_managed';
    if (engineId === 'none') return 'unavailable';
    if (capabilitiesError) return 'error';

    return profiles.some(profile =>
        profile.engine_id === engineId && profile.searchable
    )
        ? 'available'
        : 'unavailable';
}

export function effectiveOnboardingInferenceChoice(
    choice: InferenceChoice,
    localSupport: OnboardingLocalInferenceSupport,
): InferenceChoice {
    return localSupport === 'unavailable' ? 'cloud' : choice;
}

export function isOnboardingLocalInferenceSelectable(
    localSupport: OnboardingLocalInferenceSupport,
): boolean {
    return localSupport === 'available' || localSupport === 'externally_managed';
}

export function isOnboardingOllamaSelectionReady({
    status,
    models,
    selectedModel,
}: {
    status: OllamaModelsStatus;
    models: readonly string[];
    selectedModel: string | null | undefined;
}): boolean {
    return status === 'ready'
        && isInstalledOllamaModel(models, selectedModel);
}

export function buildOnboardingSteps({
    mode,
    inference,
    showEngineSetup,
}: {
    mode: 'local' | 'remote';
    inference: InferenceChoice;
    showEngineSetup: boolean;
}): Step[] {
    const steps: Step[] = ['welcome', 'style', 'mode'];
    if (mode === 'remote') steps.push('remote_setup');
    steps.push('agent', 'inference');
    if (inference === 'local' && showEngineSetup) steps.push('engine_setup');
    steps.push(inference === 'local' ? 'models' : 'api_keys', 'permissions', 'complete');
    return steps;
}

export function buildAgentSettingsPatch(agentName: string, personalityPack: string) {
    return {
        'agent.name': agentName.trim(),
        'agent.personality_pack': personalityPack,
        'agent.persona_seed': personalityPack,
    };
}

export async function persistOnboardingEmbeddingDimension<E>({
    dimension,
    currentDimension,
    persist,
}: {
    dimension: number;
    currentDimension: number | undefined;
    persist: () => Promise<
        { status: 'ok'; data: null }
        | { status: 'error'; error: E }
    >;
}): Promise<boolean> {
    if (dimension <= 0 || currentDimension === dimension) return false;
    unwrapResult(await persist(), 'save embedding dimension');
    return true;
}

const PERSONALITY_PACKS = [
    { id: 'balanced', label: 'Balanced', description: 'Clear, thoughtful, and adaptable' },
    { id: 'professional', label: 'Professional', description: 'Precise, structured, and concise' },
    { id: 'creative_partner', label: 'Creative partner', description: 'Exploratory, generative, and expressive' },
    { id: 'research_assistant', label: 'Research assistant', description: 'Evidence-oriented and analytical' },
] as const;

const ONBOARDING_PIPELINE_FILTERS: Record<ModelCategory, { label: string; task: HfModelTask; placeholder: string }> = {
    llm: { label: 'LLM (Chat & Reasoning)', task: 'chat', placeholder: 'Search LLMs... (e.g. llama, qwen, gemma)' },
    embedding: { label: 'Embedding (RAG)', task: 'embedding', placeholder: 'Search embedding models... (e.g. bge, nomic, mxbai)' },
    stt: { label: 'Speech-to-Text', task: 'stt', placeholder: 'Search speech models... (e.g. whisper)' },
    diffusion: { label: 'Image Generation', task: 'diffusion', placeholder: 'Search image generation models... (e.g. flux)' },
};

export function isOnboardingModelCategoryReady({
    enabled,
    installed,
    repoId,
    artifactId,
    companionId,
    plan,
}: {
    enabled: boolean;
    installed: boolean;
    repoId?: string | null;
    artifactId?: string | null;
    companionId?: string | null;
    plan?: HfModelFilePlanLike;
}): boolean {
    if (!enabled) return true;
    if (!repoId) return installed;
    if (!artifactId || !plan) return false;
    if (!plan.artifacts.some(artifact => artifact.id === artifactId)) return false;
    return !requiresHfCompanionArtifact(plan)
        || Boolean(effectiveHfCompanionArtifactId(plan, companionId));
}

/** Normalise a user-typed URL/IP into a clean http://host:port URL */
function normaliseHttpUrl(raw: string): string {
    let url = raw.trim();
    url = url.replace(/^wss?:\/\//, '');
    if (!/^https?:\/\//.test(url)) url = `http://${url}`;
    const withoutProto = url.replace(/^https?:\/\//, '');
    const hostPart = withoutProto.split('/')[0];
    if (!hostPart.includes(':')) url = url.replace(hostPart, `${hostPart}:3000`);
    return url;
}

// Cloud providers shown in API keys step
const CLOUD_PROVIDERS = [
    { id: 'anthropic', label: 'Anthropic', desc: 'Claude Fable 5, Opus 4.8, Sonnet 5 & Haiku 4.5', placeholder: 'sk-ant-api03-...', color: 'text-purple-500', keyUrl: 'https://console.anthropic.com/settings/keys', save: 'thinclawSaveAnthropicKey' as const },
    { id: 'openai', label: 'OpenAI', desc: 'GPT-5, reasoning & coding models', placeholder: 'sk-...', color: 'text-emerald-500', keyUrl: 'https://platform.openai.com/api-keys', save: 'thinclawSaveOpenaiKey' as const },
    { id: 'gemini', label: 'Google Gemini', desc: 'Gemini Flash, Pro & frontier models', placeholder: 'AIza...', color: 'text-cyan-500', keyUrl: 'https://aistudio.google.com/app/apikey', save: 'thinclawSaveGeminiKey' as const },
    { id: 'groq', label: 'Groq', desc: 'Ultra-fast Llama, Mixtral inference', placeholder: 'gsk_...', color: 'text-orange-400', keyUrl: 'https://console.groq.com/keys', save: 'thinclawSaveGroqKey' as const },
    { id: 'openrouter', label: 'OpenRouter', desc: 'Universal access to 100+ models', placeholder: 'sk-or-v1-...', color: 'text-indigo-500', keyUrl: 'https://openrouter.ai/keys', save: 'thinclawSaveOpenrouterKey' as const },
];

export function OnboardingWizard({ onComplete }: OnboardingWizardProps) {
    const [step, setStep] = useState<Step>('welcome');
    const [mode, setMode] = useState<'local' | 'remote'>('local');
    const [permissions, setPermissions] = useState({
        accessibility: false,
        screen_recording: false
    });
    const [isLoading, setIsLoading] = useState(false);
    const [activeInstall, setActiveInstall] = useState<{ downloadId: string; repoId: string } | null>(null);
    const [inferenceChoice, setInferenceChoice] = useState<InferenceChoice>('local');
    const [agentName, setAgentName] = useState('ThinClaw');
    const [personalityPack, setPersonalityPack] = useState('balanced');

    // --- Remote setup state ---
    const [remoteDeployMode, setRemoteDeployMode] = useState<'new' | 'existing'>('existing');
    const [remoteIp, setRemoteIp] = useState('');
    const [remoteUser, setRemoteUser] = useState('root');
    const [remoteExistingUrl, setRemoteExistingUrl] = useState('');
    const [remoteExistingToken, setRemoteExistingToken] = useState('');
    const [remoteConnecting, setRemoteConnecting] = useState(false);
    const [remoteConnected, setRemoteConnected] = useState(false);
    const [remoteDeploying, setRemoteDeploying] = useState(false);
    const [remoteDeployLogs, setRemoteDeployLogs] = useState<string[]>([]);
    const [remoteError, setRemoteError] = useState('');
    const [remoteTailscaleKey, setRemoteTailscaleKey] = useState('');
    const [remoteEnableSystemd, setRemoteEnableSystemd] = useState(true);

    // --- Per-category HF model selections ---
    const [categoryEnabled, setCategoryEnabled] = useState<Record<ModelCategory, boolean>>({ llm: true, embedding: true, stt: false, diffusion: false });
    const normalizedCategoryEngineIdRef = useRef<string | null>(null);
    const previousAvailableCategoriesRef = useRef<ModelCategory[]>([]);
    const [categorySelectedModel, setCategorySelectedModel] = useState<Record<string, string | null>>({});
    const [categorySelectedArtifact, setCategorySelectedArtifact] = useState<Record<string, string | null>>({});
    const [categorySelectedCompanion, setCategorySelectedCompanion] = useState<Record<string, string | null>>({});
    const [categoryTopModels, setCategoryTopModels] = useState<Record<string, HfModelCard[]>>({});
    const [categoryTopStatus, setCategoryTopStatus] = useState<Record<string, 'idle' | 'loading' | 'ready' | 'error'>>({});
    const [categorySearchQuery, setCategorySearchQuery] = useState<Record<string, string>>({});
    const [categorySearchResults, setCategorySearchResults] = useState<Record<string, HfModelCard[]>>({});
    const [categorySearching, setCategorySearching] = useState<Record<string, boolean>>({});
    const [categorySearchError, setCategorySearchError] = useState<Record<string, string | null>>({});
    const [categoryShowSearch, setCategoryShowSearch] = useState<Record<string, boolean>>({});
    const categoryDebounceTimers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
    const categorySearchGuards = useRef<Record<ModelCategory, ReturnType<typeof createRequestGenerationGuard>>>({
        llm: createRequestGenerationGuard(),
        embedding: createRequestGenerationGuard(),
        stt: createRequestGenerationGuard(),
        diffusion: createRequestGenerationGuard(),
    });
    const categoryTopGuards = useRef<Record<ModelCategory, ReturnType<typeof createRequestGenerationGuard>>>({
        llm: createRequestGenerationGuard(),
        embedding: createRequestGenerationGuard(),
        stt: createRequestGenerationGuard(),
        diffusion: createRequestGenerationGuard(),
    });
    const categoryPlanGuards = useRef<Record<ModelCategory, ReturnType<typeof createRequestGenerationGuard>>>({
        llm: createRequestGenerationGuard(),
        embedding: createRequestGenerationGuard(),
        stt: createRequestGenerationGuard(),
        diffusion: createRequestGenerationGuard(),
    });
    const categorySelectedModelRef = useRef(categorySelectedModel);

    // --- Cloud API keys state ---
    const [apiKeys, setApiKeys] = useState<Record<string, string>>({});
    const [apiKeySaving, setApiKeySaving] = useState<Record<string, boolean>>({});
    const [apiKeySaved, setApiKeySaved] = useState<Record<string, boolean>>({});

    const [hfToken, setHfToken] = useState<string>('');

    // ---------------------------------------------------------------------------
    // Engine setup hook + HF Hub state (for MLX/vLLM LLM selection)
    // ---------------------------------------------------------------------------
    const engineSetup = useEngineSetup();

    const [hfCapabilities, setHfCapabilities] = useState<HfCapabilityProfileDto[]>([]);
    const [hfCapabilitiesLoading, setHfCapabilitiesLoading] = useState(true);
    const [hfCapabilitiesError, setHfCapabilitiesError] = useState<string | null>(null);
    const [hfCapabilitiesEngineId, setHfCapabilitiesEngineId] = useState<string | null>(null);
    const [hfCapabilitiesAttempt, setHfCapabilitiesAttempt] = useState(0);
    // Plans are task-specific even when the same repository appears in more than one category.
    const [hfFilePlanCache, setHfFilePlanCache] = useState<Record<string, HfModelFilePlan>>({});
    const [hfFilePlanErrors, setHfFilePlanErrors] = useState<Record<string, string>>({});

    const {
        theme: uiTheme,
        setTheme: setUiTheme,
        appThemeId,
        setAppThemeId
    } = useTheme();

    const currentMode = useMemo(() => {
        if (uiTheme === 'system') {
            return window.matchMedia("(prefers-color-scheme: dark)").matches ? 'dark' : 'light';
        }
        return uiTheme as 'dark' | 'light';
    }, [uiTheme]);

    // Access Model Context to trigger downloads and set paths
    const {

        modelsDir,
        setModelPath,
        setImageGenModelPath,
        setEmbeddingModelPath,
        setSttModelPath,
        downloadHfSelection,
        cancelDownload,
        downloading,
        discoveryState,
        engineInfo,
        localModels,
        currentModelPath,
        currentEmbeddingModelPath,
        currentSttModelPath,
        currentImageGenModelPath,
    } = useModelContext();
    const isOllama = engineInfo?.id === 'ollama';
    const {
        models: ollamaModels,
        status: ollamaModelsStatus,
        error: ollamaModelsError,
        refresh: refreshOllamaModels,
    } = useOllamaModels(isOllama);
    const [selectedOllamaModel, setSelectedOllamaModel] = useState<string | null>(null);

    useEffect(() => {
        if (!isOllama || ollamaModelsStatus !== 'ready') return;
        setSelectedOllamaModel(previous => chooseInstalledOllamaModel(
            ollamaModels,
            isInstalledOllamaModel(ollamaModels, previous)
                ? previous
                : currentModelPath,
        ));
    }, [currentModelPath, isOllama, ollamaModels, ollamaModelsStatus]);

    const installedModelByCategory = useMemo(() => {
        const choose = (category: string, configuredPath: string) =>
            localModels.find(model =>
                model.category === category
                && model.compatible
                && model.path === configuredPath
            ) ?? localModels.find(model => model.category === category && model.compatible);
        return {
            llm: choose('LLM', currentModelPath),
            embedding: choose('Embedding', currentEmbeddingModelPath),
            stt: choose('STT', currentSttModelPath),
            diffusion: choose('Diffusion', currentImageGenModelPath),
        };
    }, [
        currentEmbeddingModelPath,
        currentImageGenModelPath,
        currentModelPath,
        currentSttModelPath,
        localModels,
    ]);
    const hasLlmInstalled = Boolean(installedModelByCategory.llm);
    const hasEmbeddingInstalled = Boolean(installedModelByCategory.embedding);
    const hasSttInstalled = Boolean(installedModelByCategory.stt);
    const hasDiffusionInstalled = Boolean(installedModelByCategory.diffusion);

    // Engine-derived flags
    const showEngineSetupStep = engineInfo?.id === 'mlx' || engineInfo?.id === 'vllm';
    const profileForCategory = useMemo(() => {
        const profiles = {} as Partial<Record<ModelCategory, HfCapabilityProfileDto>>;
        for (const category of ['llm', 'embedding', 'stt', 'diffusion'] as ModelCategory[]) {
            const task = ONBOARDING_PIPELINE_FILTERS[category].task;
            const profile = hfCapabilities.find(candidate =>
                candidate.engine_id === engineInfo?.id
                && candidate.task === task
                && candidate.searchable
            );
            if (profile) profiles[category] = profile;
        }
        return profiles;
    }, [engineInfo?.id, hfCapabilities]);
    const availableModelCategories = useMemo(
        () => (['llm', 'embedding', 'stt', 'diffusion'] as ModelCategory[])
            .filter(category => Boolean(profileForCategory[category])),
        [profileForCategory],
    );
    const localInferenceSupport = useMemo(
        () => resolveOnboardingLocalInferenceSupport({
            engineId: engineInfo?.id,
            profiles: hfCapabilities,
            capabilitiesLoading:
                hfCapabilitiesLoading
                || hfCapabilitiesEngineId !== engineInfo?.id,
            capabilitiesError: hfCapabilitiesError,
        }),
        [
            engineInfo?.id,
            hfCapabilities,
            hfCapabilitiesEngineId,
            hfCapabilitiesError,
            hfCapabilitiesLoading,
        ],
    );
    const effectiveInferenceChoice = effectiveOnboardingInferenceChoice(
        inferenceChoice,
        localInferenceSupport,
    );
    const ollamaSelectionReady = isOnboardingOllamaSelectionReady({
        status: ollamaModelsStatus,
        models: ollamaModels,
        selectedModel: selectedOllamaModel,
    });
    const localInferenceSelectable =
        isOnboardingLocalInferenceSelectable(localInferenceSupport)
        && (!isOllama || ollamaSelectionReady);
    const inferenceSelectionReady =
        effectiveInferenceChoice === 'cloud' || localInferenceSelectable;

    useEffect(() => {
        if (localInferenceSupport !== 'unavailable') return;
        setInferenceChoice('cloud');
        setStep(current =>
            current === 'engine_setup' || current === 'models'
                ? 'inference'
                : current
        );
    }, [localInferenceSupport]);

    const modelSelectionsReady = useMemo(() => {
        if (effectiveInferenceChoice !== 'local') return true;
        if (!localInferenceSelectable) return false;
        if (localInferenceSupport === 'externally_managed') {
            return ollamaSelectionReady;
        }
        if (hfCapabilitiesLoading || hfCapabilitiesError || !engineInfo) return false;
        if (profileForCategory.llm && !categoryEnabled.llm) return false;
        const installed: Record<ModelCategory, boolean> = {
            llm: hasLlmInstalled,
            embedding: hasEmbeddingInstalled,
            stt: hasSttInstalled,
            diffusion: hasDiffusionInstalled,
        };
        return availableModelCategories.every(category => {
            const repoId = categorySelectedModel[category];
            return isOnboardingModelCategoryReady({
                enabled: categoryEnabled[category],
                installed: installed[category],
                repoId,
                artifactId: categorySelectedArtifact[category],
                companionId: categorySelectedCompanion[category],
                plan: repoId
                    ? hfFilePlanCache[hfPlanKey(category, repoId)]
                    : undefined,
            });
        });
    }, [
        availableModelCategories,
        categoryEnabled,
        categorySelectedArtifact,
        categorySelectedCompanion,
        categorySelectedModel,
        hasDiffusionInstalled,
        hasEmbeddingInstalled,
        hasLlmInstalled,
        hasSttInstalled,
        hfFilePlanCache,
        hfCapabilitiesError,
        hfCapabilitiesLoading,
        effectiveInferenceChoice,
        engineInfo,
        localInferenceSelectable,
        localInferenceSupport,
        ollamaSelectionReady,
        profileForCategory.llm,
    ]);

    // Dynamic step list based on user choices
    const stepList = useMemo(() => buildOnboardingSteps({
        mode,
        inference: effectiveInferenceChoice,
        showEngineSetup: showEngineSetupStep,
    }), [showEngineSetupStep, mode, effectiveInferenceChoice]);

    const progressPct = useMemo(() => {
        const idx = stepList.indexOf(step);
        return ((idx + 1) / stepList.length) * 100;
    }, [step, stepList]);

    // Suppress ModelProvider first-run toast during onboarding
    useEffect(() => {
        startOnboardingProgress();
        return () => { clearOnboardingProgress(); };
    }, []);

    useEffect(() => {
        let cancelled = false;
        const requestedEngineId = engineInfo?.id ?? null;
        setHfCapabilitiesLoading(true);
        setHfCapabilitiesEngineId(null);
        setHfCapabilitiesError(null);
        directCommands.directRuntimeGetHfCapabilities()
            .then(profiles => {
                if (!cancelled) {
                    setHfCapabilities(profiles);
                    setHfCapabilitiesEngineId(requestedEngineId);
                }
            })
            .catch(error => {
                console.error('Failed to load Hugging Face capabilities:', error);
                if (!cancelled) {
                    setHfCapabilities([]);
                    setHfCapabilitiesError(bridgeErrorMessage(error));
                    setHfCapabilitiesEngineId(requestedEngineId);
                }
            })
            .finally(() => {
                if (!cancelled) setHfCapabilitiesLoading(false);
            });
        return () => {
            cancelled = true;
        };
    }, [engineInfo?.id, hfCapabilitiesAttempt]);

    useEffect(() => {
        categorySelectedModelRef.current = categorySelectedModel;
    }, [categorySelectedModel]);

    useEffect(() => {
        const engineId = engineInfo?.id ?? null;
        const authoritative = Boolean(
            engineId
            && !hfCapabilitiesLoading
            && !hfCapabilitiesError
            && hfCapabilitiesEngineId === engineId
        );
        if (!authoritative || !engineId) return;

        const resetDefaults = normalizedCategoryEngineIdRef.current !== engineId;
        const previousAvailableCategories = resetDefaults
            ? []
            : previousAvailableCategoriesRef.current;
        setCategoryEnabled(previous =>
            reconcileOnboardingHfCategoryEnabled({
                previous,
                availableCategories: availableModelCategories,
                previousAvailableCategories,
                authoritative,
                resetDefaults,
            })
        );
        normalizedCategoryEngineIdRef.current = engineId;
        previousAvailableCategoriesRef.current = [...availableModelCategories];
    }, [
        availableModelCategories,
        engineInfo?.id,
        hfCapabilitiesEngineId,
        hfCapabilitiesError,
        hfCapabilitiesLoading,
    ]);

    // Load one revision-pinned artifact plan only when the user selects a model.
    const loadHfFilePlan = useCallback(async (
        category: ModelCategory,
        repoId: string,
    ): Promise<HfModelFilePlan | undefined> => {
        const key = hfPlanKey(category, repoId);
        const cached = hfFilePlanCache[key];
        if (cached) return cached;
        const profile = profileForCategory[category];
        if (!profile) return undefined;
        const generation = categoryPlanGuards.current[category].begin();
        try {
            if (hfToken.trim()) {
                await thinclaw.setHfToken(hfToken.trim());
            }
            setHfFilePlanErrors(previous => {
                const next = { ...previous };
                delete next[key];
                return next;
            });
            const plan = unwrapResult(
                await directCommands.directRuntimeGetModelFilesV2(repoId, profile.task),
                'Hugging Face artifact plan'
            );
            setHfFilePlanCache(previous => ({ ...previous, [key]: plan }));
            const recommended = selectRecommendedArtifact(plan.artifacts);
            if (
                recommended
                && categoryPlanGuards.current[category].isCurrent(generation)
                && categorySelectedModelRef.current[category] === repoId
            ) {
                setCategorySelectedArtifact(previous => ({
                    ...previous,
                    [category]: recommended.id,
                }));
                const companionId = effectiveHfCompanionArtifactId(plan);
                if (companionId) {
                    setCategorySelectedCompanion(previous => ({
                        ...previous,
                        [category]: companionId,
                    }));
                }
            }
            return plan;
        } catch (error) {
            const message = bridgeErrorMessage(error);
            setHfFilePlanErrors(previous => ({ ...previous, [key]: message }));
            console.error('Failed to load Hugging Face artifact plan:', error);
            return undefined;
        }
    }, [hfFilePlanCache, hfToken, profileForCategory]);

    useEffect(() => {
        checkPermissions();
        const interval = setInterval(checkPermissions, 2000);
        return () => clearInterval(interval);
    }, []);

    const checkPermissions = async () => {
        try {
            const perms = await thinclaw.getPermissionStatus();
            setPermissions(perms);
        } catch (e) {
            console.error("Failed to check permissions", e);
        }
    };

    const handleNext = () => {
        if (step === 'agent' && !agentName.trim()) return;
        if (step === 'inference' && !inferenceSelectionReady) return;
        if (step === 'engine_setup' && engineSetup.status?.state !== 'ready') return;
        if (step === 'complete') { handleFinish(); return; }
        const idx = stepList.indexOf(step);
        if (idx < stepList.length - 1) setStep(stepList[idx + 1]);
    };

    // ---------------------------------------------------------------------------
    // Per-category HF model loading (loads top models when entering models step)
    // ---------------------------------------------------------------------------
    const loadCategoryModels = useCallback(async (cat: ModelCategory) => {
        const profile = profileForCategory[cat];
        if (
            !engineInfo
            || !profile
            || !shouldStartHfTopModelsRequest(categoryTopStatus[cat])
        ) return;
        const generation = categoryTopGuards.current[cat].begin();
        setCategoryTopStatus(previous => ({ ...previous, [cat]: 'loading' }));
        try {
            const response = unwrapResult(
                await directCommands.directRuntimeDiscoverHfModelsV2('', profile.task, 5),
                `HuggingFace ${cat} models`
            );
            if (!categoryTopGuards.current[cat].isCurrent(generation)) return;
            setCategoryTopModels(prev => ({ ...prev, [cat]: response.models }));
            setCategoryTopStatus(previous => ({ ...previous, [cat]: 'ready' }));
        } catch (err) {
            if (categoryTopGuards.current[cat].isCurrent(generation)) {
                console.error(`Failed to load top ${cat} models:`, err);
                setCategoryTopStatus(previous => ({ ...previous, [cat]: 'error' }));
            }
        }
    }, [engineInfo, categoryTopStatus, profileForCategory]);

    // Load top models for all enabled categories when entering models step
    useEffect(() => {
        if (step !== 'models' || !engineInfo) return;
        availableModelCategories.forEach(cat => {
            if (categoryEnabled[cat]) loadCategoryModels(cat);
        });
    }, [step, engineInfo, categoryEnabled, availableModelCategories, loadCategoryModels]);

    // Per-category debounced search
    const searchCategory = useCallback((cat: ModelCategory, query: string) => {
        setCategorySearchQuery(prev => ({ ...prev, [cat]: query }));
        categorySearchGuards.current[cat].invalidate();
        if (categoryDebounceTimers.current[cat]) clearTimeout(categoryDebounceTimers.current[cat]);
        if (!query.trim() || !engineInfo) {
            setCategorySearchResults(prev => ({ ...prev, [cat]: [] }));
            setCategorySearching(prev => ({ ...prev, [cat]: false }));
            setCategorySearchError(prev => ({ ...prev, [cat]: null }));
            return;
        }
        categoryDebounceTimers.current[cat] = setTimeout(async () => {
            const profile = profileForCategory[cat];
            if (!profile) return;
            const generation = categorySearchGuards.current[cat].begin();
            setCategorySearching(prev => ({ ...prev, [cat]: true }));
            setCategorySearchError(prev => ({ ...prev, [cat]: null }));
            try {
                const response = unwrapResult(
                    await directCommands.directRuntimeDiscoverHfModelsV2(query, profile.task, 10),
                    `HuggingFace ${cat} search`
                );
                if (!categorySearchGuards.current[cat].isCurrent(generation)) return;
                setCategorySearchResults(prev => ({ ...prev, [cat]: response.models }));
            } catch (error) {
                if (categorySearchGuards.current[cat].isCurrent(generation)) {
                    setCategorySearchError(prev => ({
                        ...prev,
                        [cat]: bridgeErrorMessage(error),
                    }));
                }
            } finally {
                if (categorySearchGuards.current[cat].isCurrent(generation)) {
                    setCategorySearching(prev => ({ ...prev, [cat]: false }));
                }
            }
        }, 350);
    }, [engineInfo, profileForCategory]);

    useEffect(() => () => {
        for (const timer of Object.values(categoryDebounceTimers.current)) {
            clearTimeout(timer);
        }
        for (const guard of Object.values(categorySearchGuards.current)) {
            guard.invalidate();
        }
        for (const guard of Object.values(categoryTopGuards.current)) {
            guard.invalidate();
        }
        for (const guard of Object.values(categoryPlanGuards.current)) {
            guard.invalidate();
        }
    }, []);

    // ---------------------------------------------------------------------------
    // Remote agent connection handler
    // ---------------------------------------------------------------------------
    const handleRemoteConnect = async () => {
        if (!remoteExistingUrl) return;
        const url = normaliseHttpUrl(remoteExistingUrl);
        setRemoteConnecting(true);
        setRemoteError('');
        try {
            const ok = unwrapResult(
                await commands.thinclawTestConnection(url, remoteExistingToken || null),
                'Test remote gateway connection',
            );
            if (!ok) {
                setRemoteError('Cannot connect — server unreachable or auth failed');
                setRemoteConnecting(false);
                return;
            }
            const displayHost = url.replace(/^https?:\/\//, '').split(':')[0];
            const newProfile: thinclaw.AgentProfile = {
                id: crypto.randomUUID(),
                name: `Remote (${displayHost})`,
                url,
                token: remoteExistingToken || null,
                mode: 'remote',
                auto_connect: true,
            };
            await thinclaw.addAgentProfile(newProfile);
            unwrapResult(
                await commands.thinclawSaveGatewaySettings('remote', url, remoteExistingToken || ''),
                'Save remote gateway settings',
            );
            setRemoteConnected(true);
            toast.success('Connected to remote agent!');
        } catch (e: any) {
            setRemoteError(typeof e === 'string' ? e : e.message || 'Connection failed');
        } finally {
            setRemoteConnecting(false);
        }
    };

    // Remote deploy handler
    const handleRemoteDeploy = async () => {
        if (!remoteIp) return;
        setRemoteDeploying(true);
        setRemoteDeployLogs(['=== ThinClaw Remote Deploy ===', `Target: ${remoteUser}@${remoteIp}`]);
        setRemoteError('');
        try {
            const unlistenLog = await listen<string>('deploy-log', (event) => {
                setRemoteDeployLogs((prev) => [...prev, event.payload]);
            });
            try {
                const result = unwrapResult(
                    await commands.thinclawDeployRemote(
                        remoteIp,
                        remoteUser,
                        remoteTailscaleKey || null,
                        remoteEnableSystemd,
                    ),
                    'Deploy remote gateway',
                );
                const newProfile: thinclaw.AgentProfile = {
                    id: crypto.randomUUID(),
                    name: `Remote (${remoteIp})`,
                    url: result.url,
                    token: result.token || null,
                    mode: 'remote',
                    auto_connect: true,
                };
                // Save the returned credential even if the health check timed
                // out: the deployment may still finish after this command, and
                // the one-time token must not be lost with the response object.
                await thinclaw.addAgentProfile(newProfile);
                unwrapResult(
                    await commands.thinclawSaveGatewaySettings('remote', result.url, result.token || ''),
                    'Save deployed gateway settings',
                );
                if (result.status === 'success') {
                    setRemoteConnected(true);
                    toast.success('Remote agent deployed and connected!');
                } else {
                    setRemoteError(`${result.message || 'Deployment health check timed out'} The URL and credential were saved; retry the connection when the gateway is ready.`);
                }
            } finally {
                unlistenLog();
            }
        } catch (e: any) {
            setRemoteError(typeof e === 'string' ? e : e.message);
        } finally {
            setRemoteDeploying(false);
        }
    };

    // ---------------------------------------------------------------------------
    // Cloud API key save handler
    // ---------------------------------------------------------------------------
    const handleSaveApiKey = async (providerId: string) => {
        const value = apiKeys[providerId]?.trim();
        if (!value) return;
        setApiKeySaving(prev => ({ ...prev, [providerId]: true }));
        try {
            const provider = CLOUD_PROVIDERS.find(p => p.id === providerId);
            if (!provider) return;
            const res = await (commands as any)[provider.save](value);
            if (res?.status === 'ok') {
                setApiKeySaved(prev => ({ ...prev, [providerId]: true }));
                setApiKeys(prev => ({ ...prev, [providerId]: '' }));
                toast.success(`${provider.label} key saved`);
            } else {
                toast.error(`Failed to save ${provider.label} key`);
            }
        } catch {
            toast.error('Failed to save API key');
        } finally {
            setApiKeySaving(prev => ({ ...prev, [providerId]: false }));
        }
    };

    // ---------------------------------------------------------------------------
    // handleFinish — orchestrates all final saves
    // ---------------------------------------------------------------------------
    const handleFinish = async () => {
        setIsLoading(true);
        try {
            if (effectiveInferenceChoice === 'local' && !localInferenceSelectable) {
                throw new Error(
                    isOllama
                        ? 'Install a model in Ollama, refresh the library, and choose it before enabling local inference.'
                        : localInferenceSupport === 'error'
                        ? 'Local runtime support could not be verified. Retry the capability check or choose cloud inference.'
                        : 'Local runtime capabilities are still being checked.'
                );
            }

            unwrapResult(
                await commands.thinclawConfigPatch(
                    buildAgentSettingsPatch(agentName, personalityPack)
                ),
                'agent setup'
            );

            // Save HF Token if provided
            if (hfToken && hfToken.trim().length > 0) {
                await thinclaw.setHfToken(hfToken.trim());
            }

            // --- Local inference: download selected HF models per category ---
            if (
                effectiveInferenceChoice === 'local'
                && localInferenceSupport !== 'externally_managed'
            ) {
                if (hfCapabilitiesLoading || hfCapabilitiesError || !engineInfo) {
                    throw new Error('Runtime model capabilities are not ready.');
                }
                if (availableModelCategories.length === 0) {
                    throw new Error('This runtime has no supported local model installation workflow.');
                }
                if (profileForCategory.llm && !categoryEnabled.llm) {
                    throw new Error('A local chat model is required.');
                }
                const categoriesToInstall = availableModelCategories.filter(cat =>
                    categoryEnabled[cat] && Boolean(categorySelectedModel[cat])
                );
                for (const cat of availableModelCategories) {
                    if (
                        categoryEnabled[cat]
                        && !installedModelByCategory[cat]
                        && !categorySelectedModel[cat]
                    ) {
                        throw new Error(`Choose a ${ONBOARDING_PIPELINE_FILTERS[cat].label} model before finishing setup.`);
                    }
                }

                const installSelections = [];
                for (const cat of categoriesToInstall) {
                    const repoId = categorySelectedModel[cat]!;
                    const plan = hfFilePlanCache[hfPlanKey(cat, repoId)]
                        ?? await loadHfFilePlan(cat, repoId);
                    if (!plan) {
                        throw new Error(`Could not resolve a downloadable artifact for ${repoId}.`);
                    }
                    const artifactId = categorySelectedArtifact[cat];
                    const artifact = plan.artifacts.find(candidate => candidate.id === artifactId);
                    if (!artifact) {
                        throw new Error(`Choose a model artifact for ${repoId} before finishing setup.`);
                    }

                    const companionId = effectiveHfCompanionArtifactId(
                        plan,
                        categorySelectedCompanion[cat],
                    );
                    const companion = companionId
                        ? plan.companion_artifacts.find(candidate => candidate.id === companionId)
                        : null;
                    if (companionId && !companion) {
                        throw new Error(`The selected vision projector for ${repoId} is no longer available.`);
                    }
                    if (requiresHfCompanionArtifact(plan) && !companion) {
                        throw new Error(`Choose a vision projector for ${repoId} before finishing setup.`);
                    }
                    const installed = findInstalledArtifactSelection(localModels, {
                        repoId,
                        revision: plan.revision,
                        engineId: plan.engine_id,
                        task: plan.task,
                        artifactId: artifact.id,
                        companionArtifactId: companion?.id ?? null,
                    });
                    installSelections.push({
                        category: cat,
                        plan,
                        artifact,
                        companion,
                        existingModelPath: installed?.path ?? null,
                    });
                }

                // Existing models only count when their concrete path is
                // applied. This repairs reset/stale preferences instead of
                // treating "present on disk" as "configured".
                const setCategoryPath = (category: ModelCategory, modelPath: string) => {
                    if (category === 'llm') setModelPath(modelPath);
                    else if (category === 'embedding') setEmbeddingModelPath(modelPath);
                    else if (category === 'stt') setSttModelPath(modelPath);
                    else if (category === 'diffusion') setImageGenModelPath(modelPath);
                };
                for (const cat of availableModelCategories) {
                    if (
                        categoryEnabled[cat]
                        && !categorySelectedModel[cat]
                        && installedModelByCategory[cat]
                    ) {
                        setCategoryPath(cat, installedModelByCategory[cat]!.path);
                    }
                }

                const embeddingSelection = installSelections.find(
                    selection => selection.category === 'embedding'
                );

                await installOnboardingHfSelections({
                    selections: installSelections,
                    download: async (request, downloadId) => {
                        setActiveInstall({ downloadId, repoId: request.repo_id });
                        try {
                            return await downloadHfSelection(request, downloadId);
                        } finally {
                            setActiveInstall(null);
                        }
                    },
                    setPath: setCategoryPath,
                });

                // Only change vector-store dimensions after the selected
                // embedding install has succeeded. Existing managed installs
                // use their own pinned provenance; legacy files stay unchanged.
                const installedEmbedding = installedModelByCategory.embedding;
                const embeddingProvenance = embeddingSelection
                    ? {
                        repoId: embeddingSelection.plan.repo_id,
                        revision: embeddingSelection.plan.revision,
                    }
                    : categoryEnabled.embedding
                        && installedEmbedding?.repo_id
                        && installedEmbedding.revision
                        ? {
                            repoId: installedEmbedding.repo_id,
                            revision: installedEmbedding.revision,
                        }
                        : null;
                if (embeddingProvenance) {
                    let discoveredDimension: number | null = null;
                    try {
                        discoveredDimension = unwrapResult(
                            await directCommands.directRuntimeDiscoverEmbeddingDimension(
                                embeddingProvenance.repoId,
                                embeddingProvenance.revision,
                            ),
                            'HuggingFace embedding dimension'
                        );
                    } catch (e) {
                        console.warn('[onboarding] Could not discover embedding dimension:', e);
                    }
                    if (discoveredDimension && discoveredDimension > 0) {
                        const userConfig = await commands.getUserConfig();
                        await persistOnboardingEmbeddingDimension({
                            dimension: discoveredDimension,
                            currentDimension: userConfig.vector_dimensions,
                            persist: () => commands.updateUserConfig({
                                vector_dimensions: discoveredDimension,
                            }),
                        });
                    }
                }
            }
            if (
                effectiveInferenceChoice === 'local'
                && localInferenceSupport === 'externally_managed'
            ) {
                const currentOllamaModels = await refreshOllamaModels();
                if (!currentOllamaModels) {
                    throw new Error(
                        'Could not verify the Ollama model library. Start Ollama and refresh before finishing setup.'
                    );
                }
                if (!isInstalledOllamaModel(currentOllamaModels, selectedOllamaModel)) {
                    throw new Error(
                        'The selected Ollama model is no longer installed. Refresh the library and choose an installed model.'
                    );
                }
                setModelPath(selectedOllamaModel);
            }

            // Persist the inference mode only after all required local installs
            // and their post-install metadata updates have succeeded.
            await thinclaw.toggleThinClawLocalInference(
                effectiveInferenceChoice === 'local'
            );

            // Save setup completed status
            await thinclaw.setSetupCompleted(true);
            clearOnboardingProgress();
            toast.success("Setup complete!");
            onComplete();
        } catch (e) {
            console.error('[onboarding] Failed to finish setup:', e);
            toast.error("Failed to finish setup", {
                description: bridgeErrorMessage(e),
            });
        } finally {
            setActiveInstall(null);
            setIsLoading(false);
        }
    };

    return (
        <div
            role="dialog"
            aria-modal="true"
            aria-labelledby="onboarding-dialog-title"
            className="fixed inset-0 z-50 bg-background/95 backdrop-blur-xs flex items-center justify-center p-4"
        >
            <motion.div
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ opacity: 1, scale: 1 }}
                className="w-full max-w-4xl bg-card border border-border rounded-xl shadow-2xl overflow-hidden flex flex-col max-h-[90vh]"
            >
                <h1 id="onboarding-dialog-title" className="sr-only">ThinClaw Desktop setup</h1>
                <Progress value={progressPct} label={`Setup step ${stepList.indexOf(step) + 1} of ${stepList.length}`} />

                <div className="p-8 flex-1 overflow-y-auto" aria-live="polite">
                    <AnimatePresence mode="wait">
                        {step === 'welcome' && (
                            <motion.div
                                key="welcome"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6 text-center"
                            >
                                <div className="w-16 h-16 bg-primary/10 rounded-2xl flex items-center justify-center mx-auto mb-6">
                                    <Globe className="w-8 h-8 text-primary" />
                                </div>
                                <h1 className="text-3xl font-bold tracking-tight">Welcome to ThinClaw Desktop</h1>
                                <p className="text-lg text-muted-foreground max-w-md mx-auto">
                                    Your secure, private, and open-source AI desktop. Let's configure your ThinClaw experience.
                                </p>
                            </motion.div>
                        )}

                        {step === 'style' && (
                            <motion.div
                                key="style"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-8"
                            >
                                <div className="text-center">
                                    <h2 className="text-2xl font-bold font-display">Workspace Aesthetics</h2>
                                    <p className="text-muted-foreground">Personalize your environment. Choose a theme that fits your workflow.</p>
                                </div>

                                <div className="grid md:grid-cols-2 gap-8 items-start">
                                    <div className="space-y-4">
                                        <div className="flex items-center justify-between px-1">
                                            <span className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">Select Theme</span>
                                            <div className="flex gap-1 p-0.5 bg-muted/20 rounded-lg border border-border/50">
                                                <button
                                                    onClick={() => setUiTheme("light")}
                                                    className={cn("p-1.5 rounded-md transition-all", currentMode === 'light' ? 'bg-background shadow-xs text-primary' : 'text-muted-foreground hover:text-foreground')}
                                                    aria-label="Use light appearance"
                                                    aria-pressed={currentMode === 'light'}
                                                >
                                                    <Sun className="w-3.5 h-3.5" />
                                                </button>
                                                <button
                                                    onClick={() => setUiTheme("dark")}
                                                    className={cn("p-1.5 rounded-md transition-all", currentMode === 'dark' ? 'bg-background shadow-xs text-primary' : 'text-muted-foreground hover:text-foreground')}
                                                    aria-label="Use dark appearance"
                                                    aria-pressed={currentMode === 'dark'}
                                                >
                                                    <Moon className="w-3.5 h-3.5" />
                                                </button>
                                            </div>
                                        </div>

                                        <div className="grid grid-cols-2 gap-3">
                                            {APP_THEMES.map((t) => {
                                                const isActive = appThemeId === t.id;
                                                const colors = currentMode === 'dark' ? t.dark : t.light;
                                                return (
                                                    <button
                                                        key={t.id}
                                                        onClick={() => setAppThemeId(t.id)}
                                                        aria-label={`Use ${t.label} palette`}
                                                        aria-pressed={isActive}
                                                        className={cn(
                                                            "group p-3 rounded-xl border-2 text-left transition-all space-y-3",
                                                            isActive
                                                                ? "border-primary bg-primary/5 shadow-md"
                                                                : "border-border hover:border-primary/50 bg-card"
                                                        )}
                                                    >
                                                        <div className="flex items-center justify-between">
                                                            <span className="text-xs font-bold">{t.label}</span>
                                                            {isActive && <CheckCircle className="w-3 h-3 text-primary" />}
                                                        </div>
                                                        <div className="flex gap-1.5 p-1 rounded-lg w-full border border-border/10 justify-center" style={{ backgroundColor: `hsl(${colors.background})` }}>
                                                            <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.primary})` }} />
                                                            <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.accent})` }} />
                                                            <div className="w-3 h-3 rounded-full border border-black/10 dark:border-white/10" style={{ backgroundColor: `hsl(${colors.secondary})` }} />
                                                        </div>
                                                    </button>
                                                );
                                            })}
                                        </div>
                                    </div>

                                    {/* Real-time Preview Area */}
                                    <div className="space-y-4">
                                        <span className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 px-1">Live Preview</span>
                                        <div className="aspect-4/3 rounded-2xl border border-border bg-card shadow-2xl overflow-hidden flex flex-col relative group">
                                            {/* Window Controls */}
                                            <div className="h-8 bg-muted/40 border-b border-border/50 flex items-center px-3 gap-1.5">
                                                <div className="w-2.5 h-2.5 rounded-full bg-rose-500/20" />
                                                <div className="w-2.5 h-2.5 rounded-full bg-amber-500/20" />
                                                <div className="w-2.5 h-2.5 rounded-full bg-emerald-500/20" />
                                            </div>

                                            {/* Mock Chat Interface */}
                                            <div className="flex-1 p-4 space-y-4 overflow-hidden">
                                                <div className="flex gap-2">
                                                    <div className="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
                                                        <Cpu className="w-4 h-4 text-primary" />
                                                    </div>
                                                    <div className="space-y-2 flex-1">
                                                        <div className="h-2 bg-primary/20 rounded-full w-1/2" />
                                                        <div className="h-2 bg-muted rounded-full w-3/4" />
                                                        <div className="h-2 bg-muted rounded-full w-2/3" />
                                                    </div>
                                                </div>
                                                <div className="flex gap-2 justify-end">
                                                    <div className="space-y-2 flex-1 items-end flex flex-col">
                                                        <div className="h-2 bg-secondary rounded-full w-1/3" />
                                                    </div>
                                                    <div className="w-8 h-8 rounded-lg bg-secondary flex items-center justify-center shrink-0">
                                                        <div className="w-4 h-4 rounded-full bg-primary/40" />
                                                    </div>
                                                </div>
                                                <div className="flex gap-2">
                                                    <div className="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
                                                        <Cpu className="w-4 h-4 text-primary" />
                                                    </div>
                                                    <div className="space-y-2 flex-1">
                                                        <div className="h-2 bg-muted rounded-full w-5/6" />
                                                        <div className="h-2 bg-muted rounded-full w-1/2" />
                                                    </div>
                                                </div>
                                            </div>

                                            {/* Mock Input */}
                                            <div className="p-4 border-t border-border/50 bg-muted/10 h-16 flex items-center gap-2 mt-auto">
                                                <div className="flex-1 h-8 rounded-lg bg-background border border-border" />
                                                <div className="w-8 h-8 rounded-lg bg-primary" />
                                            </div>

                                            {/* Hover indicator for "Great UX" feel */}
                                            <div className="absolute inset-0 pointer-events-none bg-linear-to-t from-primary/5 to-transparent opacity-0 group-hover:opacity-100 transition-opacity duration-700" />
                                        </div>

                                        <div className="flex items-center gap-3 p-4 bg-primary/5 rounded-xl border border-primary/10 text-xs text-muted-foreground leading-relaxed animate-in fade-in slide-in-from-bottom-2 duration-500">
                                            <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center shrink-0">
                                                <Palette className="w-4 h-4 text-primary" />
                                            </div>
                                            <span>The entire interface updates in real-time as you switch themes. You can always refine these in Settings later.</span>
                                        </div>
                                    </div>
                                </div>
                            </motion.div>
                        )}

                        {step === 'mode' && (
                            <motion.div
                                key="mode"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6"
                            >
                                <div className="text-center mb-8">
                                    <h2 className="text-2xl font-bold">Agent Deployment</h2>
                                    <p className="text-muted-foreground">Where should your ThinClaw agent run? <span className="text-xs opacity-70 block mt-1">You can change this later in Settings &gt; Gateway.</span></p>
                                </div>

                                <div className="grid md:grid-cols-2 gap-4">
                                    <button
                                        onClick={() => setMode('local')}
                                        className={cn(
                                            "p-6 rounded-xl border-2 text-left transition-all space-y-4",
                                            mode === 'local'
                                                ? "border-primary bg-primary/5 shadow-md"
                                                : "border-border hover:border-primary/50 bg-card"
                                        )}
                                    >
                                        <div className="w-10 h-10 rounded-lg bg-blue-500/10 flex items-center justify-center text-blue-500">
                                            <Cpu className="w-6 h-6" />
                                        </div>
                                        <div>
                                            <h3 className="font-semibold text-lg">Local Agent</h3>
                                            <p className="text-sm text-muted-foreground mt-1">
                                                ThinClaw runs on this machine. Private, secure, and offline-capable.
                                            </p>
                                        </div>
                                    </button>

                                    <button
                                        onClick={() => setMode('remote')}
                                        className={cn(
                                            "p-6 rounded-xl border-2 text-left transition-all space-y-4",
                                            mode === 'remote'
                                                ? "border-primary bg-primary/5 shadow-md"
                                                : "border-border hover:border-primary/50 bg-card"
                                        )}
                                    >
                                        <div className="w-10 h-10 rounded-lg bg-purple-500/10 flex items-center justify-center text-purple-500">
                                            <Server className="w-6 h-6" />
                                        </div>
                                        <div>
                                            <h3 className="font-semibold text-lg">Remote Agent</h3>
                                            <p className="text-sm text-muted-foreground mt-1">
                                                Deploy or connect to a remote ThinClaw server. Ideal for lighter devices.
                                            </p>
                                        </div>
                                    </button>
                                </div>
                            </motion.div>
                        )}

                        {step === 'agent' && (
                            <motion.div
                                key="agent"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-7"
                            >
                                <div className="text-center">
                                    <div className="mx-auto mb-4 grid size-12 place-items-center rounded-xl bg-primary/10 text-primary">
                                        <Bot className="size-6" aria-hidden="true" />
                                    </div>
                                    <h2 className="text-2xl font-bold">Meet your agent</h2>
                                    <p className="mt-1 text-muted-foreground">
                                        This identity follows the agent in both Workbench and Agent Cockpit.
                                    </p>
                                </div>

                                <div className="mx-auto max-w-2xl space-y-5">
                                    <div className="space-y-2">
                                        <label htmlFor="onboarding-agent-name" className="text-sm font-semibold">Agent name</label>
                                        <input
                                            id="onboarding-agent-name"
                                            value={agentName}
                                            maxLength={48}
                                            onChange={(event) => setAgentName(event.currentTarget.value)}
                                            placeholder="ThinClaw"
                                            aria-invalid={!agentName.trim()}
                                            className="h-11 w-full rounded-xl border border-border bg-background px-4 text-sm outline-none transition-shadow focus:ring-2 focus:ring-primary/30"
                                        />
                                        <p className="text-xs text-muted-foreground">Used in conversations, status, and connected channels.</p>
                                    </div>

                                    <fieldset className="space-y-3">
                                        <legend className="text-sm font-semibold">Working style</legend>
                                        <div className="grid gap-3 sm:grid-cols-2">
                                            {PERSONALITY_PACKS.map((pack) => {
                                                const selected = personalityPack === pack.id;
                                                return (
                                                    <button
                                                        key={pack.id}
                                                        type="button"
                                                        role="radio"
                                                        aria-checked={selected}
                                                        onClick={() => setPersonalityPack(pack.id)}
                                                        className={cn(
                                                            "rounded-xl border p-4 text-left transition-colors",
                                                            selected
                                                                ? "border-primary bg-primary/5 ring-1 ring-primary/20"
                                                                : "border-border bg-card hover:bg-accent/50",
                                                        )}
                                                    >
                                                        <span className="block text-sm font-semibold">{pack.label}</span>
                                                        <span className="mt-1 block text-xs text-muted-foreground">{pack.description}</span>
                                                    </button>
                                                );
                                            })}
                                        </div>
                                    </fieldset>

                                    <div className="grid gap-3 rounded-xl border border-border bg-muted/30 p-4 text-xs sm:grid-cols-2">
                                        <div>
                                            <p className="font-semibold text-foreground">Workbench</p>
                                            <p className="mt-1 text-muted-foreground">Direct chat, projects, models, and private desktop workflows.</p>
                                        </div>
                                        <div>
                                            <p className="font-semibold text-foreground">Agent Cockpit</p>
                                            <p className="mt-1 text-muted-foreground">Sessions, tools, approvals, automations, and channels.</p>
                                        </div>
                                        <p className="text-muted-foreground sm:col-span-2">
                                            Channel credentials remain optional and can be added safely after setup from Cockpit → Channels.
                                        </p>
                                    </div>
                                </div>
                            </motion.div>
                        )}

                        {step === 'remote_setup' && (
                            <motion.div
                                key="remote_setup"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6"
                            >
                                <div className="text-center mb-6">
                                    <h2 className="text-2xl font-bold">Remote Agent Setup</h2>
                                    <p className="text-muted-foreground">Deploy a new agent or connect to an existing one.</p>
                                </div>

                                {/* Tab switcher */}
                                <div className="flex bg-muted p-1.5 rounded-xl">
                                    <button
                                        onClick={() => setRemoteDeployMode('existing')}
                                        className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${remoteDeployMode === 'existing' ? 'bg-background text-foreground shadow-xs' : 'text-muted-foreground hover:text-foreground'}`}
                                    >
                                        Connect Existing
                                    </button>
                                    <button
                                        onClick={() => setRemoteDeployMode('new')}
                                        className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${remoteDeployMode === 'new' ? 'bg-background text-foreground shadow-xs' : 'text-muted-foreground hover:text-foreground'}`}
                                    >
                                        Deploy New Agent
                                    </button>
                                </div>

                                {remoteDeployMode === 'existing' ? (
                                    <div className="space-y-4 animate-in fade-in slide-in-from-right-4 duration-300">
                                        <div className="bg-emerald-500/10 border border-emerald-500/20 rounded-xl p-4 text-sm text-emerald-600 dark:text-emerald-400">
                                            <h4 className="font-bold mb-1 flex items-center gap-2"><CheckCircle className="w-4 h-4" /> Direct Connection</h4>
                                            <p className="opacity-90 text-xs font-medium">Connect to an already running ThinClaw HTTP gateway.</p>
                                        </div>

                                        <div className="space-y-2">
                                            <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Agent URL / IP</label>
                                            <div className="relative">
                                                <input type="text"
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-hidden transition-all font-mono pl-10 placeholder:text-muted-foreground/50"
                                                    placeholder="192.168.1.50 or https://your-server.com:3000"
                                                    value={remoteExistingUrl}
                                                    onChange={(e) => setRemoteExistingUrl(e.target.value)}
                                                />
                                                <Server className="absolute left-3 top-3.5 w-4 h-4 text-muted-foreground" />
                                            </div>
                                            <p className="text-[10px] text-muted-foreground font-medium">Port <code>3000</code> is added automatically if omitted.</p>
                                        </div>

                                        <div className="space-y-2">
                                            <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Auth Token</label>
                                            <input type="password"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-hidden transition-all font-mono placeholder:text-muted-foreground/50"
                                                placeholder="From GATEWAY_AUTH_TOKEN in your .env"
                                                value={remoteExistingToken}
                                                onChange={(e) => setRemoteExistingToken(e.target.value)}
                                            />
                                        </div>

                                        <button
                                            onClick={handleRemoteConnect}
                                            disabled={!remoteExistingUrl || remoteConnecting}
                                            className="w-full py-3 rounded-xl bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm font-bold shadow-lg shadow-emerald-500/20 transition-all flex items-center justify-center gap-2"
                                        >
                                            {remoteConnecting ? <><Loader2 className="w-4 h-4 animate-spin" /> Testing...</> : <><Zap className="w-4 h-4" /> Test & Connect</>}
                                        </button>

                                        {remoteConnected && (
                                            <div className="flex items-center gap-3 p-4 rounded-xl bg-emerald-500/5 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400 animate-in fade-in duration-300">
                                                <CheckCircle2 className="w-5 h-5 shrink-0" />
                                                <span className="text-sm font-medium">Connected successfully! Click Next to continue.</span>
                                            </div>
                                        )}
                                    </div>
                                ) : (
                                    <div className="space-y-4 animate-in fade-in slide-in-from-left-4 duration-300">
                                        <div className="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 text-sm text-blue-600 dark:text-blue-400">
                                            <h4 className="font-bold mb-1 flex items-center gap-2"><AlertTriangle className="w-4 h-4" /> Prerequisites</h4>
                                            <ul className="list-disc list-inside space-y-1 opacity-90 text-xs font-medium">
                                                <li>A fresh Ubuntu 22+ / Debian 12 Linux server.</li>
                                                <li>SSH access configured (key-based recommended).</li>
                                                <li>Docker, UFW Firewall & Fail2ban will be installed automatically.</li>
                                            </ul>
                                        </div>

                                        <div className="grid gap-4">
                                            <div className="space-y-2">
                                                <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Server IP Address</label>
                                                <input type="text"
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-hidden transition-all font-mono placeholder:text-muted-foreground/50"
                                                    placeholder="e.g. 192.168.1.50 or your-server.com"
                                                    value={remoteIp}
                                                    onChange={(e) => setRemoteIp(e.target.value)}
                                                />
                                            </div>
                                            <div className="space-y-2">
                                                <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">SSH User</label>
                                                <input type="text"
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-hidden transition-all font-mono placeholder:text-muted-foreground/50"
                                                    placeholder="root"
                                                    value={remoteUser}
                                                    onChange={(e) => setRemoteUser(e.target.value)}
                                                />
                                            </div>
                                            <div className="space-y-2">
                                                <label className="text-xs font-semibold text-muted-foreground">Tailscale Auth Key <span className="text-muted-foreground/60">(optional)</span></label>
                                                <input type="password"
                                                    autoComplete="off"
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-hidden transition-all font-mono placeholder:text-muted-foreground/50"
                                                    placeholder="tskey-auth-..."
                                                    value={remoteTailscaleKey}
                                                    onChange={(e) => setRemoteTailscaleKey(e.target.value)}
                                                />
                                            </div>
                                            <label className="flex items-center gap-3 cursor-pointer group">
                                                <input type="checkbox" checked={remoteEnableSystemd}
                                                    onChange={(e) => setRemoteEnableSystemd(e.target.checked)}
                                                    className="w-4 h-4 rounded border-border text-primary focus:ring-primary/20"
                                                />
                                                <span className="text-sm font-medium text-foreground group-hover:text-primary transition-colors">
                                                    Create systemd service <span className="text-muted-foreground text-xs">(auto-start on boot)</span>
                                                </span>
                                            </label>
                                        </div>

                                        <button
                                            onClick={handleRemoteDeploy}
                                            disabled={!remoteIp || remoteDeploying}
                                            className="w-full py-3 rounded-xl bg-blue-600 hover:bg-blue-500 disabled:opacity-50 disabled:cursor-not-allowed text-white text-sm font-bold shadow-lg shadow-blue-500/20 transition-all flex items-center justify-center gap-2"
                                        >
                                            {remoteDeploying ? <><Loader2 className="w-4 h-4 animate-spin" /> Deploying...</> : <><Server className="w-4 h-4" /> Start Deployment</>}
                                        </button>

                                        {remoteDeploying && (
                                            <div className="bg-black/90 rounded-xl border border-border/50 p-4 font-mono text-[10px] leading-relaxed overflow-y-auto max-h-[200px] shadow-inner">
                                                {remoteDeployLogs.map((log, i) => (
                                                    <div key={i} className={`mb-0.5 whitespace-pre-wrap ${log.includes('[stderr]') ? 'text-amber-400' : 'text-zinc-400'}`}>{log}</div>
                                                ))}
                                            </div>
                                        )}

                                        {remoteConnected && (
                                            <div className="flex items-center gap-3 p-4 rounded-xl bg-emerald-500/5 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400 animate-in fade-in duration-300">
                                                <CheckCircle2 className="w-5 h-5 shrink-0" />
                                                <span className="text-sm font-medium">Agent deployed and connected! Click Next to continue.</span>
                                            </div>
                                        )}
                                    </div>
                                )}

                                {remoteError && (
                                    <div className="flex items-center gap-3 p-4 rounded-xl bg-rose-500/5 border border-rose-500/20 text-rose-600 dark:text-rose-400 animate-in fade-in duration-200">
                                        <AlertTriangle className="w-5 h-5 shrink-0" />
                                        <span className="text-sm">{remoteError}</span>
                                    </div>
                                )}
                            </motion.div>
                        )}

                        {step === 'inference' && (
                            <motion.div
                                key="inference"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6"
                            >
                                <div className="text-center mb-8">
                                    <h2 className="text-2xl font-bold">Intelligence Source</h2>
                                    <p className="text-muted-foreground">How should your AI models run? <span className="text-xs opacity-70 block mt-1">You can use both local and cloud models — configure more in Settings later.</span></p>
                                </div>

                                {localInferenceSupport === 'checking' && (
                                    <div className="flex items-center gap-3 rounded-xl border border-border/60 bg-muted/20 p-4 text-sm text-muted-foreground">
                                        <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
                                        Checking this runtime for supported local model workflows…
                                    </div>
                                )}
                                {localInferenceSupport === 'error' && (
                                    <div className="rounded-xl border border-destructive/20 bg-destructive/5 p-4 text-sm">
                                        <div className="flex items-center gap-2 font-semibold text-destructive">
                                            <AlertTriangle className="h-4 w-4 shrink-0" />
                                            Local runtime support could not be verified
                                        </div>
                                        <p className="mt-1 text-muted-foreground">{hfCapabilitiesError}</p>
                                        <button
                                            onClick={() => setHfCapabilitiesAttempt(attempt => attempt + 1)}
                                            className="mt-2 font-semibold text-primary"
                                        >
                                            Retry capability check
                                        </button>
                                    </div>
                                )}
                                {localInferenceSupport === 'unavailable' && (
                                    <div className="flex items-start gap-3 rounded-xl border border-indigo-500/20 bg-indigo-500/5 p-4 text-sm">
                                        <Info className="mt-0.5 h-4 w-4 shrink-0 text-indigo-500" />
                                        <div>
                                            <p className="font-semibold">Cloud inference selected</p>
                                            <p className="mt-1 text-muted-foreground">
                                                {engineInfo?.id === 'none'
                                                    ? 'This desktop build has no local inference runtime.'
                                                    : `${engineInfo?.display_name ?? 'The active runtime'} exposes no supported Hugging Face model installation workflows.`}
                                            </p>
                                        </div>
                                    </div>
                                )}
                                {localInferenceSupport === 'externally_managed' && (
                                    <div className="flex items-start gap-3 rounded-xl border border-blue-500/20 bg-blue-500/5 p-4 text-sm">
                                        <Info className="mt-0.5 h-4 w-4 shrink-0 text-blue-500" />
                                        <div className="min-w-0 flex-1">
                                            <p className="font-semibold">Ollama manages local models separately</p>
                                            <p className="mt-1 text-muted-foreground">
                                                {ollamaModelsStatus === 'loading'
                                                    ? 'Checking the local Ollama model library…'
                                                    : ollamaModelsError
                                                        ? ollamaModelsError
                                                        : ollamaModels.length === 0
                                                            ? 'No models are installed. Run ollama pull <model>, then refresh before choosing Local.'
                                                            : `Using ${selectedOllamaModel}. You can change this on the next step.`}
                                            </p>
                                            <button
                                                type="button"
                                                onClick={() => void refreshOllamaModels()}
                                                disabled={ollamaModelsStatus === 'loading'}
                                                className="mt-3 inline-flex items-center gap-1.5 font-semibold text-primary disabled:opacity-50"
                                            >
                                                <RefreshCw className={cn(
                                                    "h-3.5 w-3.5",
                                                    ollamaModelsStatus === 'loading' && "animate-spin",
                                                )} />
                                                Refresh Ollama library
                                            </button>
                                        </div>
                                    </div>
                                )}

                                <div className="grid md:grid-cols-2 gap-4">
                                    <button
                                        onClick={() => setInferenceChoice('local')}
                                        disabled={!localInferenceSelectable}
                                        className={cn(
                                            "relative p-6 rounded-xl border-2 text-left transition-all space-y-4 overflow-hidden group",
                                            effectiveInferenceChoice === 'local'
                                                ? "bg-emerald-500/5 border-emerald-500/50 shadow-lg shadow-emerald-500/10"
                                                : "bg-card border-border hover:border-emerald-500/30 hover:bg-emerald-500/5",
                                            !localInferenceSelectable && "cursor-not-allowed opacity-60 hover:border-border hover:bg-card",
                                        )}
                                    >
                                        <div className="flex items-start justify-between">
                                            <div className={cn("p-3 rounded-xl transition-colors",
                                                effectiveInferenceChoice === 'local' ? "bg-emerald-500 text-white" : "bg-muted text-muted-foreground group-hover:text-emerald-500"
                                            )}>
                                                <Cpu className="w-6 h-6" />
                                            </div>
                                            {effectiveInferenceChoice === 'local' && <div className="px-2 py-1 rounded-full bg-emerald-500 text-white text-[10px] font-bold uppercase tracking-wider">Selected</div>}
                                        </div>
                                        <div>
                                            <h3 className="text-lg font-bold">Local Inference</h3>
                                            <p className="text-xs text-muted-foreground mt-1">
                                                Run models directly on your device. Zero data egress, full privacy.
                                            </p>
                                        </div>
                                        <div className="flex items-center gap-2 text-[10px] font-medium text-emerald-600/80">
                                            {localInferenceSupport === 'checking'
                                                ? <Loader2 className="h-3 w-3 animate-spin" />
                                                : localInferenceSelectable
                                                    ? <div className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
                                                    : <AlertTriangle className="h-3 w-3" />}
                                            {localInferenceSupport === 'externally_managed'
                                                ? ollamaSelectionReady
                                                    ? 'Installed Ollama model selected'
                                                    : 'Install a model and refresh'
                                                : localInferenceSupport === 'available'
                                                    ? 'Next: Select models to download'
                                                    : localInferenceSupport === 'checking'
                                                        ? 'Checking local support'
                                                        : 'Unavailable for this runtime'}
                                        </div>
                                    </button>

                                    <button
                                        onClick={() => setInferenceChoice('cloud')}
                                        className={cn(
                                            "relative p-6 rounded-xl border-2 text-left transition-all space-y-4 overflow-hidden group",
                                            effectiveInferenceChoice === 'cloud'
                                                ? "bg-indigo-500/5 border-indigo-500/50 shadow-lg shadow-indigo-500/10"
                                                : "bg-card border-border hover:border-indigo-500/30 hover:bg-indigo-500/5"
                                        )}
                                    >
                                        <div className="flex items-start justify-between">
                                            <div className={cn("p-3 rounded-xl transition-colors",
                                                effectiveInferenceChoice === 'cloud' ? "bg-indigo-500 text-white" : "bg-muted text-muted-foreground group-hover:text-indigo-500"
                                            )}>
                                                <Globe className="w-6 h-6" />
                                            </div>
                                            {effectiveInferenceChoice === 'cloud' && <div className="px-2 py-1 rounded-full bg-indigo-500 text-white text-[10px] font-bold uppercase tracking-wider">Selected</div>}
                                        </div>
                                        <div>
                                            <h3 className="text-lg font-bold">Cloud Inference</h3>
                                            <p className="text-xs text-muted-foreground mt-1">
                                                Use powerful cloud models from Anthropic, OpenAI, Google & more.
                                            </p>
                                        </div>
                                        <div className="flex items-center gap-2 text-[10px] font-medium text-indigo-600/80">
                                            <div className="w-1.5 h-1.5 rounded-full bg-indigo-500 animate-pulse" />
                                            Next: Enter API keys
                                        </div>
                                    </button>
                                </div>
                            </motion.div>
                        )}

                        {step === 'engine_setup' && (
                            <motion.div
                                key="engine_setup"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6"
                            >
                                <div className="text-center mb-8">
                                    <h2 className="text-2xl font-bold">Engine Setup</h2>
                                    <p className="text-muted-foreground">
                                        {!engineSetup.needsSetup && !engineSetup.setupComplete
                                            ? `${engineInfo?.display_name ?? 'Inference engine'} is already configured.`
                                            : `${engineInfo?.display_name ?? 'Inference engine'} needs a one-time setup.`}
                                    </p>
                                </div>

                                {/* Already done from previous run (needsSetup=false, setupComplete=false) */}
                                {!engineSetup.needsSetup && !engineSetup.setupComplete && (
                                    <div className="flex items-center gap-3 p-6 rounded-xl bg-emerald-500/5 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400">
                                        <CheckCircle2 className="w-6 h-6 shrink-0" />
                                        <div>
                                            <h3 className="font-semibold">{engineInfo?.display_name} is already set up</h3>
                                            <p className="text-sm opacity-80 mt-1">Python environment was configured in a previous session. You can proceed to model selection.</p>
                                        </div>
                                    </div>
                                )}

                                {engineSetup.setupComplete ? (
                                    <div className="flex items-center gap-3 p-6 rounded-xl bg-emerald-500/5 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400">
                                        <CheckCircle2 className="w-6 h-6 shrink-0" />
                                        <div>
                                            <h3 className="font-semibold">{engineInfo?.display_name} is ready!</h3>
                                            <p className="text-sm opacity-80 mt-1">Python environment set up successfully. Click Next to select models.</p>
                                        </div>
                                    </div>
                                ) : (
                                    <div className={cn(
                                        "rounded-xl border overflow-hidden transition-all duration-300",
                                        engineSetup.setupError
                                            ? "bg-card/50 border-rose-500/20"
                                            : engineSetup.isSettingUp
                                                ? "bg-primary/5 border-primary/20"
                                                : "bg-card/50 border-amber-500/20"
                                    )}>
                                        <div className="p-6 space-y-4">
                                            <div className="flex items-start gap-4">
                                                {engineSetup.isSettingUp ? (
                                                    <Loader2 className="w-6 h-6 text-primary animate-spin shrink-0 mt-0.5" />
                                                ) : engineSetup.setupError ? (
                                                    <AlertTriangle className="w-6 h-6 text-destructive shrink-0 mt-0.5" />
                                                ) : (
                                                    <Wrench className="w-6 h-6 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
                                                )}
                                                <div className="flex-1">
                                                    <h3 className="font-semibold text-foreground">
                                                        {engineSetup.isSettingUp
                                                            ? `Setting up ${engineInfo?.display_name}...`
                                                            : engineSetup.setupError
                                                                ? "Setup Failed"
                                                                : `${engineInfo?.display_name} Setup Required`}
                                                    </h3>
                                                    <p className="text-sm text-muted-foreground mt-1">
                                                        {engineSetup.isSettingUp
                                                            ? engineSetup.setupMessage
                                                            : engineSetup.setupError
                                                                ? engineSetup.setupError
                                                                : `This downloads and configures a Python environment for ${engineInfo?.display_name} (~200MB). Takes about 2-3 minutes.`}
                                                    </p>
                                                </div>
                                            </div>

                                            {engineSetup.isSettingUp && (
                                                <div className="space-y-1.5">
                                                    <div className="h-2 bg-secondary rounded-full overflow-hidden">
                                                        <div
                                                            className="h-full bg-primary rounded-full transition-all duration-500 ease-out animate-pulse"
                                                            style={{
                                                                width: engineSetup.setupStage === 'creating_venv' ? '30%'
                                                                    : engineSetup.setupStage === 'installing' ? '70%' : '100%'
                                                            }}
                                                        />
                                                    </div>
                                                    <div className="flex items-center justify-between text-[10px] text-muted-foreground/60 uppercase tracking-wider">
                                                        <span className={cn("transition-colors", engineSetup.setupStage === 'creating_venv' && "text-primary font-semibold")}>
                                                            Create Environment
                                                        </span>
                                                        <span className={cn("transition-colors", engineSetup.setupStage === 'installing' && "text-primary font-semibold")}>
                                                            Install Packages
                                                        </span>
                                                        <span className={cn("transition-colors", engineSetup.setupStage === 'complete' && "text-primary font-semibold")}>
                                                            Ready
                                                        </span>
                                                    </div>
                                                </div>
                                            )}

                                            {!engineSetup.isSettingUp && (
                                                <button
                                                    onClick={engineSetup.triggerSetup}
                                                    className={cn(
                                                        "w-full py-3 px-4 rounded-xl text-sm font-bold uppercase tracking-wider",
                                                        "flex items-center justify-center gap-2 transition-all shadow-xs",
                                                        "hover:-translate-y-px active:translate-y-0",
                                                        engineSetup.setupError
                                                            ? "bg-destructive/10 text-destructive border border-destructive/30 hover:bg-destructive/20"
                                                            : "bg-primary text-primary-foreground hover:opacity-90"
                                                    )}
                                                >
                                                    <Zap className="w-4 h-4" />
                                                    {engineSetup.setupError ? "Retry Setup" : "Set Up Now"}
                                                </button>
                                            )}
                                        </div>
                                    </div>
                                )}
                            </motion.div>
                        )}

                        {step === 'models' && (
                            <motion.div
                                key="models"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6"
                            >
                                <div className="text-center mb-6">
                                    <h2 className="text-2xl font-bold">Select Local Models</h2>
                                    <p className="text-muted-foreground">
                                        {isOllama
                                            ? 'Choose an installed Ollama chat model.'
                                            : 'Choose models for each capability.'}
                                        {engineInfo && !isOllama && <span className="text-xs opacity-70 block mt-1">Filtered for {engineInfo.display_name} ({engineInfo.hf_tag?.toUpperCase() || 'compatible'} format).</span>}
                                    </p>
                                </div>

                                {/* Per-category sections */}
                                <div className="space-y-4">
                                    {isOllama && (
                                        <div className="rounded-xl border border-border/60 bg-card p-5">
                                            <div className="flex items-start justify-between gap-4">
                                                <div>
                                                    <h3 className="font-semibold">Ollama chat model</h3>
                                                    <p className="mt-1 text-xs text-muted-foreground">
                                                        ThinClaw reads identifiers from the local Ollama daemon. Raw Hugging Face downloads are not imported into Ollama.
                                                    </p>
                                                </div>
                                                <button
                                                    type="button"
                                                    onClick={() => void refreshOllamaModels()}
                                                    disabled={ollamaModelsStatus === 'loading'}
                                                    className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs font-semibold hover:bg-accent disabled:opacity-50"
                                                >
                                                    <RefreshCw className={cn(
                                                        "h-3.5 w-3.5",
                                                        ollamaModelsStatus === 'loading' && "animate-spin",
                                                    )} />
                                                    Refresh
                                                </button>
                                            </div>

                                            {ollamaModelsStatus === 'loading' ? (
                                                <div className="mt-4 flex items-center text-sm text-muted-foreground">
                                                    <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                                    Reading installed models…
                                                </div>
                                            ) : ollamaModelsError ? (
                                                <div className="mt-4 rounded-lg border border-destructive/20 bg-destructive/5 p-3 text-sm text-destructive">
                                                    {ollamaModelsError}
                                                </div>
                                            ) : ollamaModels.length === 0 ? (
                                                <div className="mt-4 rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 text-sm">
                                                    <p className="font-semibold text-amber-600 dark:text-amber-400">No Ollama models installed</p>
                                                    <p className="mt-1 text-muted-foreground">
                                                        Run <code className="font-mono">ollama pull &lt;model&gt;</code> in a terminal, then refresh this list.
                                                    </p>
                                                </div>
                                            ) : (
                                                <label className="mt-4 block text-sm font-medium">
                                                    Installed model
                                                    <select
                                                        value={selectedOllamaModel ?? ''}
                                                        onChange={event => setSelectedOllamaModel(event.target.value || null)}
                                                        className="mt-2 w-full rounded-lg border border-border bg-background px-3 py-2 text-sm"
                                                    >
                                                        {ollamaModels.map(model => (
                                                            <option key={model} value={model}>{model}</option>
                                                        ))}
                                                    </select>
                                                </label>
                                            )}
                                        </div>
                                    )}
                                    {!isOllama && hfCapabilitiesError && (
                                        <div className="rounded-xl border border-destructive/20 bg-destructive/5 p-5 text-sm">
                                            <p className="font-semibold text-destructive">Could not load runtime model capabilities</p>
                                            <p className="mt-1 text-muted-foreground">{hfCapabilitiesError}</p>
                                            <button
                                                onClick={() => setHfCapabilitiesAttempt(attempt => attempt + 1)}
                                                className="mt-3 font-semibold text-primary"
                                            >
                                                Retry
                                            </button>
                                        </div>
                                    )}
                                    {!isOllama && !hfCapabilitiesLoading && !hfCapabilitiesError && availableModelCategories.length === 0 && (
                                        <div className="rounded-xl border border-border/60 bg-muted/20 p-5 text-sm text-muted-foreground">
                                            This runtime has no supported Hugging Face installation workflows.
                                        </div>
                                    )}
                                    {!isOllama && hfCapabilitiesLoading && (
                                        <div className="flex items-center justify-center py-8 text-sm text-muted-foreground">
                                            <Loader2 className="w-4 h-4 mr-2 animate-spin" /> Loading runtime model capabilities…
                                        </div>
                                    )}
                                    {availableModelCategories.map((cat) => {
                                        const filter = ONBOARDING_PIPELINE_FILTERS[cat];
                                        const topModels = categoryTopModels[cat] || [];
                                        const topStatus = categoryTopStatus[cat] ?? 'idle';
                                        const searchResults = categorySearchResults[cat] || [];
                                        const isSearching = categorySearching[cat] || false;
                                        const searchError = categorySearchError[cat];
                                        const query = categorySearchQuery[cat] || '';
                                        const showSearch = categoryShowSearch[cat] || false;
                                        const selected = categorySelectedModel[cat];
                                        const enabled = categoryEnabled[cat];
                                        const selectedPlan = selected
                                            ? hfFilePlanCache[hfPlanKey(cat, selected)]
                                            : undefined;
                                        const selectedArtifactId = categorySelectedArtifact[cat] ?? '';
                                        const selectedCompanionId = selectedPlan
                                            ? effectiveHfCompanionArtifactId(
                                                selectedPlan,
                                                categorySelectedCompanion[cat],
                                            )
                                            : null;
                                        const companionRequired = selectedPlan
                                            ? requiresHfCompanionArtifact(selectedPlan)
                                            : false;
                                        const selectedPlanError = selected
                                            ? hfFilePlanErrors[hfPlanKey(cat, selected)]
                                            : undefined;
                                        const displayModels = query.trim() ? searchResults : topModels;
                                        const installedMap: Record<string, boolean> = {
                                            llm: hasLlmInstalled,
                                            embedding: hasEmbeddingInstalled,
                                            stt: hasSttInstalled,
                                            diffusion: hasDiffusionInstalled,
                                        };

                                        return (
                                            <div key={cat} className={cn(
                                                "rounded-xl border overflow-hidden transition-all duration-300",
                                                enabled ? "border-primary/20 bg-card/50" : "border-border/50 bg-muted/20 opacity-60"
                                            )}>
                                                {/* Category header with toggle */}
                                                <button
                                                    onClick={() => {
                                                        if (cat !== 'llm') {
                                                            setCategoryEnabled(prev => ({ ...prev, [cat]: !prev[cat] }));
                                                        }
                                                    }}
                                                    disabled={cat === 'llm'}
                                                    className="w-full p-4 flex items-center justify-between hover:bg-primary/5 transition-colors"
                                                >
                                                    <div className="flex items-center gap-3">
                                                        <div className={cn("p-2 rounded-lg", enabled ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground")}>
                                                            {cat === 'llm' && <Type className="w-4 h-4" />}
                                                            {cat === 'embedding' && <Database className="w-4 h-4" />}
                                                            {cat === 'stt' && <Mic className="w-4 h-4" />}
                                                            {cat === 'diffusion' && <Image className="w-4 h-4" />}
                                                        </div>
                                                        <div className="text-left">
                                                            <h4 className="text-sm font-bold">{filter.label}</h4>
                                                            {installedMap[cat] && <span className="text-[9px] font-semibold text-emerald-500 bg-emerald-500/10 px-1.5 py-0.5 rounded-full ml-2">Installed</span>}
                                                            {cat === 'llm' && <span className="text-[9px] font-semibold text-primary bg-primary/10 px-1.5 py-0.5 rounded-full ml-2">Required</span>}
                                                        </div>
                                                    </div>
                                                    <div className={cn(
                                                        "w-10 h-5 rounded-full transition-colors flex items-center px-0.5",
                                                        enabled ? "bg-primary" : "bg-muted"
                                                    )}>
                                                        <div className={cn(
                                                            "w-4 h-4 rounded-full bg-white shadow-xs transition-transform",
                                                            enabled ? "translate-x-5" : "translate-x-0"
                                                        )} />
                                                    </div>
                                                </button>

                                                {/* Model list (only when enabled) */}
                                                <AnimatePresence>
                                                    {enabled && (
                                                        <motion.div
                                                            initial={{ height: 0, opacity: 0 }}
                                                            animate={{ height: "auto", opacity: 1 }}
                                                            exit={{ height: 0, opacity: 0 }}
                                                            className="overflow-hidden"
                                                        >
                                                            <div className="px-4 pb-4 space-y-3">
                                                                {/* Search bar */}
                                                                {!showSearch ? (
                                                                    <button
                                                                        onClick={() => setCategoryShowSearch(prev => ({ ...prev, [cat]: true }))}
                                                                        className="w-full py-2 text-[10px] text-muted-foreground hover:text-primary transition-colors flex items-center justify-center gap-1.5 border border-dashed border-border/50 rounded-lg hover:border-primary/30"
                                                                    >
                                                                        <Search className="w-3 h-3" /> {filter.placeholder}
                                                                    </button>
                                                                ) : (
                                                                    <div className="relative">
                                                                        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                                                                        <input type="text"
                                                                            placeholder={filter.placeholder}
                                                                            value={query}
                                                                            onChange={(e) => searchCategory(cat, e.target.value)}
                                                                            className="w-full pl-8 pr-3 py-2 text-xs bg-background border border-border/50 rounded-lg focus:outline-hidden focus:ring-1 focus:ring-primary/20 text-foreground placeholder:text-muted-foreground/50"
                                                                            autoFocus
                                                                        />
                                                                        {isSearching && <Loader2 className="absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground animate-spin" />}
                                                                    </div>
                                                                )}

                                                                {searchError && (
                                                                    <p className="text-xs text-destructive">{searchError}</p>
                                                                )}

                                                                {/* Model list */}
                                                                <div className="space-y-1.5 max-h-[200px] overflow-y-auto pr-1">
                                                                    {displayModels.length === 0 && !isSearching && (
                                                                        <div className="text-center py-4 text-xs text-muted-foreground">
                                                                            {query.trim() ? (
                                                                                'No models found'
                                                                            ) : topStatus === 'error' ? (
                                                                                <button
                                                                                    onClick={() => {
                                                                                        setCategoryTopStatus(previous => ({ ...previous, [cat]: 'idle' }));
                                                                                    }}
                                                                                    className="font-semibold text-primary"
                                                                                >
                                                                                    Could not load models · Retry
                                                                                </button>
                                                                            ) : topStatus === 'ready' ? (
                                                                                'No compatible models found'
                                                                            ) : (
                                                                                <><Loader2 className="w-3 h-3 animate-spin inline mr-1" /> Loading trending models...</>
                                                                            )}
                                                                        </div>
                                                                    )}
                                                                    {displayModels.map(model => {
                                                                        const plan = hfFilePlanCache[hfPlanKey(cat, model.id)];
                                                                        const chosenArtifact = plan?.artifacts.find(
                                                                            artifact => artifact.id === categorySelectedArtifact[cat]
                                                                        );
                                                                        return (
                                                                            <button
                                                                                key={model.id}
                                                                                onClick={() => {
                                                                                    const cachedPlan = hfFilePlanCache[hfPlanKey(cat, model.id)];
                                                                                    const recommended = cachedPlan
                                                                                        ? selectRecommendedArtifact(cachedPlan.artifacts)
                                                                                        : null;
                                                                                    categoryPlanGuards.current[cat].invalidate();
                                                                                    categorySelectedModelRef.current = {
                                                                                        ...categorySelectedModelRef.current,
                                                                                        [cat]: model.id,
                                                                                    };
                                                                                    setCategorySelectedModel(prev => ({ ...prev, [cat]: model.id }));
                                                                                    setCategorySelectedArtifact(prev => ({
                                                                                        ...prev,
                                                                                        [cat]: recommended?.id ?? null,
                                                                                    }));
                                                                                    setCategorySelectedCompanion(prev => ({
                                                                                        ...prev,
                                                                                        [cat]: cachedPlan
                                                                                            ? effectiveHfCompanionArtifactId(cachedPlan)
                                                                                            : null,
                                                                                    }));
                                                                                    if (!cachedPlan) void loadHfFilePlan(cat, model.id);
                                                                                }}
                                                                                className={cn(
                                                                                    "w-full p-2.5 rounded-lg border text-left transition-all text-xs flex items-center gap-3",
                                                                                    selected === model.id
                                                                                        ? "border-primary bg-primary/10 shadow-xs"
                                                                                        : "border-border/50 hover:border-primary/30 bg-background/50 hover:bg-primary/5"
                                                                                )}
                                                                            >
                                                                                <div className="flex-1 min-w-0">
                                                                                    <div className="flex items-center gap-2">
                                                                                        <span className="font-bold truncate">{model.id}</span>
                                                                                        {model.gated && (
                                                                                            <span className="text-[8px] font-bold text-amber-500 bg-amber-500/10 px-1 py-0.5 rounded shrink-0">GATED</span>
                                                                                        )}
                                                                                    </div>
                                                                                    <div className="flex items-center gap-3 mt-0.5 text-muted-foreground">
                                                                                        <span className="flex items-center gap-0.5"><ArrowDownToLine className="w-2.5 h-2.5" /> {formatDownloads(model.downloads)}</span>
                                                                                        <span className="flex items-center gap-0.5"><Heart className="w-2.5 h-2.5" /> {model.likes}</span>
                                                                                        {chosenArtifact && <span className="font-mono">{chosenArtifact.total_size_display}</span>}
                                                                                    </div>
                                                                                </div>
                                                                                {selected === model.id && <CheckCircle className="w-4 h-4 text-primary shrink-0" />}
                                                                            </button>
                                                                        );
                                                                    })}
                                                                </div>

                                                                {selected && !selectedPlan && !selectedPlanError && (
                                                                    <div className="flex items-center justify-center py-3 text-xs text-muted-foreground">
                                                                        <Loader2 className="w-3.5 h-3.5 mr-2 animate-spin" /> Resolving downloadable artifacts…
                                                                    </div>
                                                                )}

                                                                {selected && selectedPlanError && (
                                                                    <div className="rounded-lg border border-destructive/20 bg-destructive/5 p-3 text-xs">
                                                                        <p className="text-destructive">{selectedPlanError}</p>
                                                                        <button
                                                                            onClick={() => void loadHfFilePlan(cat, selected)}
                                                                            className="mt-2 font-semibold text-primary"
                                                                        >
                                                                            Retry artifact lookup
                                                                        </button>
                                                                    </div>
                                                                )}

                                                                {selectedPlan && (
                                                                    <div className="rounded-lg border border-primary/20 bg-primary/5 p-3 space-y-2">
                                                                        <label className="block text-[10px] font-bold uppercase tracking-wider text-muted-foreground">
                                                                            Artifact to install
                                                                            <select
                                                                                value={selectedArtifactId}
                                                                                onChange={event => setCategorySelectedArtifact(previous => ({
                                                                                    ...previous,
                                                                                    [cat]: event.target.value || null,
                                                                                }))}
                                                                                className="mt-1.5 w-full rounded-md border border-border/60 bg-background px-2.5 py-2 text-xs normal-case tracking-normal"
                                                                            >
                                                                                <option value="">Choose an artifact…</option>
                                                                                {selectedPlan.artifacts.map(artifact => (
                                                                                    <option key={artifact.id} value={artifact.id}>
                                                                                        {artifact.label} · {artifact.total_size_display}
                                                                                        {artifact.files.length > 1 ? ` · ${artifact.files.length} shards` : ''}
                                                                                        {findInstalledArtifactSelection(localModels, {
                                                                                            repoId: selectedPlan.repo_id,
                                                                                            revision: selectedPlan.revision,
                                                                                            engineId: selectedPlan.engine_id,
                                                                                            task: selectedPlan.task,
                                                                                            artifactId: artifact.id,
                                                                                            companionArtifactId: selectedCompanionId,
                                                                                        }) ? ' · installed' : ''}
                                                                                    </option>
                                                                                ))}
                                                                            </select>
                                                                        </label>

                                                                        {selectedPlan.companion_artifacts.length > 0 && (
                                                                            <label className="block text-[10px] font-bold uppercase tracking-wider text-muted-foreground">
                                                                                Vision projector {companionRequired ? '(required)' : '(optional)'}
                                                                                <select
                                                                                    value={selectedCompanionId ?? ''}
                                                                                    onChange={event => setCategorySelectedCompanion(previous => ({
                                                                                        ...previous,
                                                                                        [cat]: event.target.value || null,
                                                                                    }))}
                                                                                    className="mt-1.5 w-full rounded-md border border-border/60 bg-background px-2.5 py-2 text-xs normal-case tracking-normal"
                                                                                >
                                                                                    {!companionRequired && (
                                                                                        <option value="">No projector</option>
                                                                                    )}
                                                                                    {selectedPlan.companion_artifacts.map(artifact => (
                                                                                        <option key={artifact.id} value={artifact.id}>
                                                                                            {artifact.label} · {artifact.total_size_display}
                                                                                        </option>
                                                                                    ))}
                                                                                </select>
                                                                            </label>
                                                                        )}

                                                                        {companionRequired && !selectedCompanionId && (
                                                                            <p className="text-[10px] text-destructive">
                                                                                This vision model has no compatible projector and cannot be installed.
                                                                            </p>
                                                                        )}

                                                                        {selectedPlan.warnings.map(warning => (
                                                                            <p key={warning} className="text-[10px] text-amber-600 dark:text-amber-400">
                                                                                {warning}
                                                                            </p>
                                                                        ))}
                                                                    </div>
                                                                )}
                                                            </div>
                                                        </motion.div>
                                                    )}
                                                </AnimatePresence>
                                            </div>
                                        );
                                    })}
                                </div>

                                {/* HF Token */}
                                <div className="space-y-2">
                                    <label className="text-[10px] font-bold text-muted-foreground uppercase tracking-[0.15em]">
                                        HuggingFace Token <span className="text-muted-foreground/60">(for gated models)</span>
                                    </label>
                                    <input
                                        type="password"
                                        value={hfToken}
                                        onChange={(e) => setHfToken(e.target.value)}
                                        onBlur={() => {
                                            if (hfToken.trim()) {
                                                void thinclaw.setHfToken(hfToken.trim()).catch(error => {
                                                    toast.error("Could not save Hugging Face token", {
                                                        description: bridgeErrorMessage(error),
                                                    });
                                                });
                                            }
                                        }}
                                        placeholder="hf_..."
                                        className="w-full bg-muted/50 border border-border rounded-xl px-4 py-2.5 text-sm focus:ring-2 focus:ring-primary/20 outline-hidden transition-all font-mono placeholder:text-muted-foreground/50"
                                    />
                                </div>

                                {!modelSelectionsReady && (
                                    <p className="text-xs text-center text-amber-600 dark:text-amber-400">
                                        Choose one model and one artifact for each enabled capability.
                                    </p>
                                )}

                                {/* Info box */}
                                <div className="bg-blue-500/10 border border-blue-500/20 rounded-lg p-4 text-sm text-blue-400 flex gap-3">
                                    <Info className="w-5 h-5 shrink-0" />
                                    <div>
                                        <p className="font-medium mb-1">Setup verifies every download</p>
                                        <p className="opacity-90">
                                            Get Started waits for the selected artifacts and stores the exact loadable paths. Failed downloads keep setup open so you can retry safely.
                                            <br />
                                            <button
                                                onClick={() => modelsDir && void commands.openModelsFolder()}
                                                className="underline hover:text-blue-300 mt-1 inline-flex items-center gap-1"
                                            >
                                                Open Models Folder <HardDrive className="w-3 h-3" />
                                            </button>
                                        </p>
                                    </div>
                                </div>
                            </motion.div>
                        )}

                        {step === 'api_keys' && (
                            <motion.div
                                key="api_keys"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6"
                            >
                                <div className="text-center mb-6">
                                    <h2 className="text-2xl font-bold">Cloud Provider Keys</h2>
                                    <p className="text-muted-foreground">Enter API keys for the providers you'd like to use. <span className="text-xs opacity-70 block mt-1">You can skip this and configure later in Settings &gt; Secrets.</span></p>
                                </div>

                                <div className="space-y-3">
                                    {CLOUD_PROVIDERS.map(provider => (
                                        <div key={provider.id} className="rounded-xl border border-border bg-card/50 p-4 space-y-3">
                                            <div className="flex items-center justify-between">
                                                <div className="flex items-center gap-3">
                                                    <Bot className={cn("w-5 h-5", provider.color)} />
                                                    <div>
                                                        <h4 className="text-sm font-bold">{provider.label}</h4>
                                                        <p className="text-[10px] text-muted-foreground">{provider.desc}</p>
                                                    </div>
                                                </div>
                                                {apiKeySaved[provider.id] && (
                                                    <span className="flex items-center gap-1 text-emerald-500 text-xs font-bold bg-emerald-500/10 px-2 py-1 rounded-full">
                                                        <CheckCircle className="w-3 h-3" /> Saved
                                                    </span>
                                                )}
                                            </div>

                                            <div className="flex gap-2">
                                                <input
                                                    type="password"
                                                    value={apiKeys[provider.id] || ''}
                                                    onChange={(e) => setApiKeys(prev => ({ ...prev, [provider.id]: e.target.value }))}
                                                    placeholder={provider.placeholder}
                                                    className="flex-1 bg-muted/50 border border-border rounded-lg px-3 py-2 text-xs font-mono focus:ring-2 focus:ring-primary/20 outline-hidden transition-all placeholder:text-muted-foreground/40"
                                                />
                                                <button
                                                    onClick={() => handleSaveApiKey(provider.id)}
                                                    disabled={!apiKeys[provider.id]?.trim() || apiKeySaving[provider.id]}
                                                    className="px-4 py-2 rounded-lg bg-primary text-primary-foreground text-xs font-bold disabled:opacity-50 disabled:cursor-not-allowed hover:bg-primary/90 transition-colors flex items-center gap-1.5"
                                                >
                                                    {apiKeySaving[provider.id] ? <Loader2 className="w-3 h-3 animate-spin" /> : <Key className="w-3 h-3" />}
                                                    Save
                                                </button>
                                            </div>
                                            <a href={provider.keyUrl} target="_blank" rel="noopener noreferrer" className="text-[10px] text-primary/70 hover:text-primary transition-colors font-medium">
                                                Get a key →
                                            </a>
                                        </div>
                                    ))}
                                </div>
                            </motion.div>
                        )}


                        {step === 'permissions' && (
                            <motion.div
                                key="permissions"
                                initial={{ opacity: 0, x: 20 }}
                                animate={{ opacity: 1, x: 0 }}
                                exit={{ opacity: 0, x: -20 }}
                                className="space-y-6"
                            >
                                <div className="text-center mb-8">
                                    <h2 className="text-2xl font-bold">Grant Permissions</h2>
                                    <p className="text-muted-foreground">ThinClaw needs access to interact with your system. <span className="text-xs opacity-70 block mt-1">These settings can be managed later.</span></p>
                                </div>

                                <div className="space-y-4">
                                    <div className="flex items-center justify-between p-4 bg-muted/50 rounded-lg border border-border">
                                        <div className="flex items-center gap-4">
                                            <div className="w-10 h-10 rounded-full bg-background flex items-center justify-center border border-border">
                                                <Code className="w-5 h-5 text-muted-foreground" />
                                            </div>
                                            <div>
                                                <h3 className="font-medium">Accessibility</h3>
                                                <p className="text-sm text-muted-foreground">Required for reading screen content and automation.</p>
                                            </div>
                                        </div>
                                        {permissions.accessibility ? (
                                            <div className="flex items-center gap-2">
                                                <span className="flex items-center gap-1.5 text-green-500 text-sm font-medium bg-green-500/10 px-3 py-1 rounded-full">
                                                    <CheckCircle className="w-4 h-4" /> Granted
                                                </span>
                                                <button
                                                    onClick={() => thinclaw.openPermissionSettings('accessibility')}
                                                    className="text-xs text-muted-foreground hover:text-foreground underline underline-offset-2 transition-colors"
                                                >
                                                    Manage
                                                </button>
                                            </div>
                                        ) : (
                                            <button
                                                onClick={async () => {
                                                    const updated = await thinclaw.requestPermission('accessibility');
                                                    setPermissions(updated);
                                                }}
                                                className="text-sm bg-primary text-primary-foreground hover:bg-primary/90 px-4 py-2 rounded-lg font-medium transition-colors"
                                            >
                                                Grant Access
                                            </button>
                                        )}
                                    </div>

                                    <div className="flex items-center justify-between p-4 bg-muted/50 rounded-lg border border-border">
                                        <div className="flex items-center gap-4">
                                            <div className="w-10 h-10 rounded-full bg-background flex items-center justify-center border border-border">
                                                <Monitor className="w-5 h-5 text-muted-foreground" />
                                            </div>
                                            <div>
                                                <h3 className="font-medium">Screen Recording</h3>
                                                <p className="text-sm text-muted-foreground">Required for seeing your screen context.</p>
                                            </div>
                                        </div>
                                        {permissions.screen_recording ? (
                                            <div className="flex items-center gap-2">
                                                <span className="flex items-center gap-1.5 text-green-500 text-sm font-medium bg-green-500/10 px-3 py-1 rounded-full">
                                                    <CheckCircle className="w-4 h-4" /> Granted
                                                </span>
                                                <button
                                                    onClick={() => thinclaw.openPermissionSettings('screen_recording')}
                                                    className="text-xs text-muted-foreground hover:text-foreground underline underline-offset-2 transition-colors"
                                                >
                                                    Manage
                                                </button>
                                            </div>
                                        ) : (
                                            <button
                                                onClick={async () => {
                                                    const updated = await thinclaw.requestPermission('screen_recording');
                                                    setPermissions(updated);
                                                }}
                                                className="text-sm bg-primary text-primary-foreground hover:bg-primary/90 px-4 py-2 rounded-lg font-medium transition-colors"
                                            >
                                                Grant Access
                                            </button>
                                        )}
                                    </div>
                                </div>
                            </motion.div>
                        )}

                        {step === 'complete' && (
                            <motion.div
                                key="complete"
                                initial={{ opacity: 0, scale: 0.9 }}
                                animate={{ opacity: 1, scale: 1 }}
                                className="space-y-6 text-center py-8"
                            >
                                <div className={cn(
                                    "w-20 h-20 rounded-full flex items-center justify-center mx-auto mb-6",
                                    isLoading ? "bg-primary/10" : "bg-green-500/10",
                                )}>
                                    {isLoading
                                        ? <Loader2 className="w-10 h-10 text-primary animate-spin" />
                                        : <CheckCircle className="w-10 h-10 text-green-500" />}
                                </div>
                                <h2 className="text-3xl font-bold">
                                    {isLoading
                                        ? activeInstall
                                            ? `Installing ${activeInstall.repoId}`
                                            : 'Finishing setup'
                                        : 'Ready to install and configure'}
                                </h2>
                                <p className="text-lg text-muted-foreground max-w-md mx-auto">
                                    {isLoading
                                        ? activeInstall
                                            ? discoveryState.repoProgress[activeInstall.downloadId]?.currentFile || 'Preparing verified artifact download…'
                                            : 'Saving your choices and validating the runtime…'
                                        : `ThinClaw will finish your ${mode} agent with ${effectiveInferenceChoice} inference when you click Get Started.`}
                                </p>
                                {activeInstall && (
                                    <div className="mx-auto max-w-md space-y-3 text-left">
                                        <Progress
                                            label="Verified download"
                                            showValue
                                            value={
                                                discoveryState.repoProgress[activeInstall.downloadId]?.pct
                                                ?? downloading[activeInstall.downloadId]
                                                ?? 0
                                            }
                                        />
                                        <button
                                            onClick={() => void cancelDownload(activeInstall.downloadId)}
                                            className="mx-auto block text-sm font-semibold text-destructive"
                                        >
                                            Cancel download
                                        </button>
                                    </div>
                                )}
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>

                <div className="p-6 border-t border-border bg-muted/10 flex justify-between items-center">
                    {step !== 'welcome' && step !== 'complete' ? (
                        <button
                            onClick={() => {
                                const idx = stepList.indexOf(step);
                                if (idx > 0) setStep(stepList[idx - 1]);
                            }}
                            className="text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
                        >
                            Back
                        </button>
                    ) : (
                        <div />
                    )}

                    <button
                        onClick={handleNext}
                        disabled={
                            isLoading
                            || (step === 'agent' && !agentName.trim())
                            || (step === 'inference' && !inferenceSelectionReady)
                            || (step === 'engine_setup' && engineSetup.status?.state !== 'ready')
                            || (step === 'models' && !modelSelectionsReady)
                        }
                        className="flex items-center gap-2 bg-primary text-primary-foreground hover:bg-primary/90 px-6 py-2.5 rounded-lg font-medium transition-all shadow-xs hover:shadow-sm"
                    >
                        {step === 'complete' ? (
                            "Get Started"
                        ) : (
                            <>
                                Next <ChevronRight className="w-4 h-4" />
                            </>
                        )}
                    </button>
                </div>
            </motion.div>
        </div>
    );
}
