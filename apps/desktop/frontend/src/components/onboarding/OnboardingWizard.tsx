import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    CheckCircle, ChevronRight, Monitor, Globe, Cpu, Code, HardDrive, Info, Palette, Moon, Sun,
    Search, Heart, ArrowDownToLine, Loader2, Zap, Wrench, AlertTriangle, CheckCircle2,
    Server, Key, Bot, Database, Image, Mic, Type
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { useTheme } from '../theme-provider';
import { APP_THEMES } from '../../lib/app-themes';
import { toast } from 'sonner';
// model-library no longer used directly — all models discovered via HF Hub
import { useModelContext } from '../model-context';
import { openPath } from '../../lib/thinclaw';
import { commands } from '../../lib/bindings';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEngineSetup } from '../../hooks/use-engine-setup';
import { clearOnboardingProgress, startOnboardingProgress } from '../../lib/local-storage-migration';

// ---------------------------------------------------------------------------
// HF Hub types (match backend via specta)
// ---------------------------------------------------------------------------
interface HfModelCard {
    id: string;
    author: string;
    name: string;
    downloads: number;
    likes: number;
    tags: string[];
    last_modified: string;
    gated: boolean;
}

interface HfFileInfo {
    filename: string;
    size: number;
    size_display: string;
    quant_type: string | null;
    is_mmproj: boolean;
}

interface ModelDownloadInfo {
    repo_id: string;
    is_multi_file: boolean;
    files: HfFileInfo[];
    mmproj_file: HfFileInfo | null;
    total_size: number;
    total_size_display: string;
}

function formatDownloads(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toString();
}

interface OnboardingWizardProps {
    onComplete: () => void;
}

type Step = 'welcome' | 'style' | 'mode' | 'remote_setup' | 'engine_setup' | 'inference' | 'models' | 'api_keys' | 'permissions' | 'complete';
type InferenceChoice = 'local' | 'cloud';
type ModelCategory = 'llm' | 'embedding' | 'stt' | 'diffusion';

// Pipeline filter definitions — matches HFDiscovery.tsx for consistency
const ONBOARDING_PIPELINE_FILTERS: Record<ModelCategory, { label: string; tags: string[]; downloadCategory: string | null; placeholder: string }> = {
    llm: { label: 'LLM (Chat & Reasoning)', tags: ['text-generation', 'image-text-to-text'], downloadCategory: null, placeholder: 'Search LLMs... (e.g. llama, qwen, gemma)' },
    embedding: { label: 'Embedding (RAG)', tags: ['feature-extraction', 'sentence-similarity'], downloadCategory: 'Embedding', placeholder: 'Search embedding models... (e.g. bge, nomic, mxbai)' },
    stt: { label: 'Speech-to-Text', tags: ['automatic-speech-recognition'], downloadCategory: 'STT', placeholder: 'Search speech models... (e.g. whisper)' },
    diffusion: { label: 'Image Generation', tags: ['text-to-image', 'image-to-image'], downloadCategory: 'Diffusion', placeholder: 'Search diffusion models... (e.g. flux, stable-diffusion)' },
};

/** Normalise a user-typed URL/IP into a clean http://host:port URL */
function normaliseHttpUrl(raw: string): string {
    let url = raw.trim();
    url = url.replace(/^wss?:\/\//, '');
    if (!/^https?:\/\//.test(url)) url = `http://${url}`;
    const withoutProto = url.replace(/^https?:\/\//, '');
    const hostPart = withoutProto.split('/')[0];
    if (!hostPart.includes(':')) url = url.replace(hostPart, `${hostPart}:18789`);
    return url;
}

// Cloud providers shown in API keys step
const CLOUD_PROVIDERS = [
    { id: 'anthropic', label: 'Anthropic', desc: 'Claude Sonnet, Opus & Haiku models', placeholder: 'sk-ant-api03-...', color: 'text-purple-500', keyUrl: 'https://console.anthropic.com/settings/keys', save: 'thinclawSaveAnthropicKey' as const },
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
    const [inferenceChoice, setInferenceChoice] = useState<InferenceChoice>('local');

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
    const [categorySelectedModel, setCategorySelectedModel] = useState<Record<string, string | null>>({});
    const [categoryTopModels, setCategoryTopModels] = useState<Record<string, HfModelCard[]>>({});
    const [categorySearchQuery, setCategorySearchQuery] = useState<Record<string, string>>({});
    const [categorySearchResults, setCategorySearchResults] = useState<Record<string, HfModelCard[]>>({});
    const [categorySearching, setCategorySearching] = useState<Record<string, boolean>>({});
    const [categoryShowSearch, setCategoryShowSearch] = useState<Record<string, boolean>>({});
    const categoryDebounceTimers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});

    // --- Cloud API keys state ---
    const [apiKeys, setApiKeys] = useState<Record<string, string>>({});
    const [apiKeySaving, setApiKeySaving] = useState<Record<string, boolean>>({});
    const [apiKeySaved, setApiKeySaved] = useState<Record<string, boolean>>({});

    const [hfToken, setHfToken] = useState<string>('');

    // ---------------------------------------------------------------------------
    // Engine setup hook + HF Hub state (for MLX/vLLM LLM selection)
    // ---------------------------------------------------------------------------
    const engineSetup = useEngineSetup();

    // HF file info cache (shared across per-category selections)
    const [hfFileInfoCache, setHfFileInfoCache] = useState<Record<string, ModelDownloadInfo>>({});

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
        downloadHfFiles,
        engineInfo,
        localModels,
    } = useModelContext();

    // Detect if models are already installed (for re-run wizard display)
    const hasLlmInstalled = localModels.some((m: any) => m.category === 'LLM' || m.path?.startsWith('LLM/'));
    const hasEmbeddingInstalled = localModels.some((m: any) => m.category === 'Embedding' || m.path?.startsWith('Embedding/'));
    const hasSttInstalled = localModels.some((m: any) => m.category === 'STT' || m.path?.startsWith('STT/'));
    const hasDiffusionInstalled = localModels.some((m: any) => m.category === 'Diffusion' || m.path?.startsWith('Diffusion/'));

    // Trusted HF model authors — models from these orgs are verified to work with their respective engines
    const trustedAuthors = new Set(['mlx-community', 'bartowski', 'second-state', 'TheBloke', 'meta-llama', 'Qwen', 'google', 'microsoft', 'mistralai', 'NousResearch']);

    // Engine-derived flags
    const showEngineSetupStep = engineInfo?.id === 'mlx' || engineInfo?.id === 'vllm';

    // Dynamic step list based on user choices
    const stepList = useMemo(() => {
        const s: Step[] = ['welcome', 'style', 'mode'];
        if (mode === 'remote') s.push('remote_setup');
        if (showEngineSetupStep) s.push('engine_setup');
        s.push('inference');
        if (inferenceChoice === 'local') {
            s.push('models');
        } else {
            s.push('api_keys');
        }
        s.push('permissions', 'complete');
        return s;
    }, [showEngineSetupStep, mode, inferenceChoice]);

    const progressPct = useMemo(() => {
        const idx = stepList.indexOf(step);
        return ((idx + 1) / stepList.length) * 100;
    }, [step, stepList]);

    // Suppress ModelProvider first-run toast during onboarding
    useEffect(() => {
        startOnboardingProgress();
        return () => { clearOnboardingProgress(); };
    }, []);

    // HF: load file info for a model (used by per-category selections)
    const loadHfFileInfo = useCallback(async (repoId: string) => {
        if (hfFileInfoCache[repoId] || !engineInfo) return;
        try {
            const info = await invoke<ModelDownloadInfo>('get_model_files', { repoId, engine: engineInfo.id });
            setHfFileInfoCache(prev => ({ ...prev, [repoId]: info }));
        } catch (err) { console.error('Failed to load file info:', err); }
    }, [engineInfo, hfFileInfoCache]);

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
        if (step === 'complete') { handleFinish(); return; }
        const idx = stepList.indexOf(step);
        if (idx < stepList.length - 1) setStep(stepList[idx + 1]);
    };

    // ---------------------------------------------------------------------------
    // Per-category HF model loading (loads top models when entering models step)
    // ---------------------------------------------------------------------------
    const loadCategoryModels = useCallback(async (cat: ModelCategory) => {
        if (!engineInfo || categoryTopModels[cat]?.length > 0) return;
        const filter = ONBOARDING_PIPELINE_FILTERS[cat];
        try {
            const models = await invoke<HfModelCard[]>('discover_hf_models', {
                query: '', engine: engineInfo.id, limit: 5,
                pipelineTags: filter.tags,
            });
            setCategoryTopModels(prev => ({ ...prev, [cat]: models }));
            // Auto-select first model if nothing selected yet
            if (models.length > 0 && !categorySelectedModel[cat]) {
                setCategorySelectedModel(prev => ({ ...prev, [cat]: models[0].id }));
            }
        } catch (err) {
            console.error(`Failed to load top ${cat} models:`, err);
        }
    }, [engineInfo, categoryTopModels, categorySelectedModel]);

    // Load top models for all enabled categories when entering models step
    useEffect(() => {
        if (step !== 'models' || !engineInfo) return;
        const categories: ModelCategory[] = ['llm', 'embedding', 'stt', 'diffusion'];
        categories.forEach(cat => {
            if (categoryEnabled[cat]) loadCategoryModels(cat);
        });
    }, [step, engineInfo, categoryEnabled]);

    // Per-category debounced search
    const searchCategory = useCallback((cat: ModelCategory, query: string) => {
        setCategorySearchQuery(prev => ({ ...prev, [cat]: query }));
        if (categoryDebounceTimers.current[cat]) clearTimeout(categoryDebounceTimers.current[cat]);
        if (!query.trim() || !engineInfo) {
            setCategorySearchResults(prev => ({ ...prev, [cat]: [] }));
            return;
        }
        categoryDebounceTimers.current[cat] = setTimeout(async () => {
            setCategorySearching(prev => ({ ...prev, [cat]: true }));
            try {
                const filter = ONBOARDING_PIPELINE_FILTERS[cat];
                const results = await invoke<HfModelCard[]>('discover_hf_models', {
                    query, engine: engineInfo.id, limit: 10,
                    pipelineTags: filter.tags,
                });
                setCategorySearchResults(prev => ({ ...prev, [cat]: results }));
                // Pre-fetch file info
                for (const model of results) {
                    if (!hfFileInfoCache[model.id]) {
                        invoke<ModelDownloadInfo>('get_model_files', { repoId: model.id, engine: engineInfo.id })
                            .then(info => setHfFileInfoCache(prev => ({ ...prev, [model.id]: info })))
                            .catch(() => { /* silent */ });
                    }
                }
            } catch { /* silent */ } finally {
                setCategorySearching(prev => ({ ...prev, [cat]: false }));
            }
        }, 350);
    }, [engineInfo, hfFileInfoCache]);

    // ---------------------------------------------------------------------------
    // Remote agent connection handler
    // ---------------------------------------------------------------------------
    const handleRemoteConnect = async () => {
        if (!remoteExistingUrl) return;
        const url = normaliseHttpUrl(remoteExistingUrl);
        setRemoteConnecting(true);
        setRemoteError('');
        try {
            const ok = await commands.thinclawTestConnection(url, remoteExistingToken || null);
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
            await commands.thinclawSaveGatewaySettings('remote', url, remoteExistingToken || '');
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
            const unlistenStatus = await listen<string>('deploy-status', (event) => {
                unlistenLog();
                unlistenStatus();
                try {
                    const result = JSON.parse(event.payload);
                    if (result.status === 'success') {
                        setRemoteConnected(true);
                        // Auto-save the deployed agent
                        const newProfile: thinclaw.AgentProfile = {
                            id: crypto.randomUUID(),
                            name: `Remote (${remoteIp})`,
                            url: result.url,
                            token: result.token || null,
                            mode: 'remote',
                            auto_connect: true,
                        };
                        thinclaw.addAgentProfile(newProfile).catch(console.error);
                        commands.thinclawSaveGatewaySettings('remote', result.url, result.token || '').catch(console.error);
                        toast.success('Remote agent deployed and connected!');
                    } else {
                        setRemoteError(result.message || 'Deployment failed');
                    }
                } catch {
                    setRemoteError(event.payload);
                }
                setRemoteDeploying(false);
            });
            await commands.thinclawDeployRemote(remoteIp, remoteUser, remoteTailscaleKey || null, remoteEnableSystemd);
        } catch (e: any) {
            setRemoteError(typeof e === 'string' ? e : e.message);
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
            // Save HF Token if provided
            if (hfToken && hfToken.trim().length > 0) {
                await thinclaw.setHfToken(hfToken.trim());
            }

            // Clear onboarding flag
            clearOnboardingProgress();

            // Set inference mode
            await thinclaw.toggleThinClawLocalInference(inferenceChoice === 'local');

            // --- Local inference: download selected HF models per category ---
            if (inferenceChoice === 'local') {
                const categoriesToDownload: { cat: ModelCategory; repoId: string }[] = [];
                for (const cat of ['llm', 'embedding', 'stt', 'diffusion'] as ModelCategory[]) {
                    if (categoryEnabled[cat] && categorySelectedModel[cat]) {
                        categoriesToDownload.push({ cat, repoId: categorySelectedModel[cat]! });
                    }
                }

                // Pre-configure vector store dimension from selected embedding model.
                // This avoids a wasteful create-then-destroy cycle when the embedding
                // server auto-detects a different dimension on first load.
                const embeddingEntry = categoriesToDownload.find(e => e.cat === 'embedding');
                if (embeddingEntry) {
                    try {
                        const dim = await invoke<number | null>('discover_embedding_dimension', {
                            repoId: embeddingEntry.repoId,
                        });
                        if (dim && dim > 0) {
                            const userConfig = await commands.getUserConfig();
                            if (userConfig.vector_dimensions !== dim) {
                                await commands.updateUserConfig({ ...userConfig, vector_dimensions: dim });
                                console.log(`[onboarding] Pre-set vector_dimensions to ${dim} for ${embeddingEntry.repoId}`);
                            }
                        }
                    } catch (e) {
                        // Non-fatal — the embedding server will auto-detect at runtime
                        console.warn('[onboarding] Could not pre-discover embedding dimension:', e);
                    }
                }

                for (const { cat, repoId } of categoriesToDownload) {
                    const filter = ONBOARDING_PIPELINE_FILTERS[cat];
                    const downloadCat = filter.downloadCategory || 'LLM';
                    const sanitizedRepo = repoId.replace(/\//g, '_');
                    const dirPath = `${downloadCat}/${sanitizedRepo}`;

                    // Check if already downloaded
                    const alreadyDownloaded = localModels.some((m: any) => m.path?.startsWith(dirPath));
                    if (alreadyDownloaded) {
                        console.log(`Skipping download — already present: ${dirPath}`);
                    } else {
                        let fileInfo = hfFileInfoCache[repoId];
                        if (!fileInfo && engineInfo) {
                            try {
                                fileInfo = await invoke<ModelDownloadInfo>('get_model_files', {
                                    repoId, engine: engineInfo.id,
                                });
                            } catch { /* silent */ }
                        }
                        if (fileInfo) {
                            const allFiles = fileInfo.files.map(f => f.filename);
                            if (fileInfo.mmproj_file) allFiles.push(fileInfo.mmproj_file.filename);
                            downloadHfFiles(repoId, allFiles, downloadCat).catch(e =>
                                console.error(`Failed to download ${cat} model:`, e)
                            );
                        }
                    }

                    // Set the model path for this category
                    if (cat === 'llm') setModelPath(dirPath);
                    else if (cat === 'embedding') setEmbeddingModelPath(dirPath);
                    else if (cat === 'stt') setSttModelPath(dirPath);
                    else if (cat === 'diffusion') setImageGenModelPath(dirPath);
                }

                if (categoriesToDownload.length > 0) {
                    toast.success("Background downloads started. Check Settings > Models.");
                }
            }

            // Save setup completed status
            await thinclaw.setSetupCompleted(true);
            toast.success("Setup complete!");
            onComplete();
        } catch (e) {
            toast.error("Failed to save setup status");
        } finally {
            setIsLoading(false);
        }
    };

    return (
        <div className="fixed inset-0 z-50 bg-background/95 backdrop-blur-sm flex items-center justify-center p-4">
            <motion.div
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ opacity: 1, scale: 1 }}
                className="w-full max-w-4xl bg-card border border-border rounded-xl shadow-2xl overflow-hidden flex flex-col max-h-[90vh]"
            >
                <div className="h-1 bg-muted w-full">
                    <motion.div
                        className="h-full bg-primary"
                        initial={{ width: "0%" }}
                        animate={{ width: `${progressPct}%` }}
                    />
                </div>

                <div className="p-8 flex-1 overflow-y-auto">
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
                                                    className={cn("p-1.5 rounded-md transition-all", currentMode === 'light' ? 'bg-background shadow-sm text-primary' : 'text-muted-foreground hover:text-foreground')}
                                                >
                                                    <Sun className="w-3.5 h-3.5" />
                                                </button>
                                                <button
                                                    onClick={() => setUiTheme("dark")}
                                                    className={cn("p-1.5 rounded-md transition-all", currentMode === 'dark' ? 'bg-background shadow-sm text-primary' : 'text-muted-foreground hover:text-foreground')}
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
                                        <div className="aspect-[4/3] rounded-2xl border border-border bg-card shadow-2xl overflow-hidden flex flex-col relative group">
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
                                            <div className="absolute inset-0 pointer-events-none bg-gradient-to-t from-primary/5 to-transparent opacity-0 group-hover:opacity-100 transition-opacity duration-700" />
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
                                        className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${remoteDeployMode === 'existing' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
                                    >
                                        Connect Existing
                                    </button>
                                    <button
                                        onClick={() => setRemoteDeployMode('new')}
                                        className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${remoteDeployMode === 'new' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
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
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono pl-10 placeholder:text-muted-foreground/50"
                                                    placeholder="192.168.1.50 or http://your-server.com:18789"
                                                    value={remoteExistingUrl}
                                                    onChange={(e) => setRemoteExistingUrl(e.target.value)}
                                                />
                                                <Server className="absolute left-3 top-3.5 w-4 h-4 text-muted-foreground" />
                                            </div>
                                            <p className="text-[10px] text-muted-foreground font-medium">Port <code>18789</code> is added automatically if omitted.</p>
                                        </div>

                                        <div className="space-y-2">
                                            <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Auth Token</label>
                                            <input type="password"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
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
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                                    placeholder="e.g. 192.168.1.50 or your-server.com"
                                                    value={remoteIp}
                                                    onChange={(e) => setRemoteIp(e.target.value)}
                                                />
                                            </div>
                                            <div className="space-y-2">
                                                <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">SSH User</label>
                                                <input type="text"
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                                    placeholder="root"
                                                    value={remoteUser}
                                                    onChange={(e) => setRemoteUser(e.target.value)}
                                                />
                                            </div>
                                            <div className="space-y-2">
                                                <label className="text-xs font-semibold text-muted-foreground">Tailscale Auth Key <span className="text-muted-foreground/60">(optional)</span></label>
                                                <input type="text"
                                                    className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
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

                                <div className="grid md:grid-cols-2 gap-4">
                                    <button
                                        onClick={() => setInferenceChoice('local')}
                                        className={cn(
                                            "relative p-6 rounded-xl border-2 text-left transition-all space-y-4 overflow-hidden group",
                                            inferenceChoice === 'local'
                                                ? "bg-emerald-500/5 border-emerald-500/50 shadow-lg shadow-emerald-500/10"
                                                : "bg-card border-border hover:border-emerald-500/30 hover:bg-emerald-500/5"
                                        )}
                                    >
                                        <div className="flex items-start justify-between">
                                            <div className={cn("p-3 rounded-xl transition-colors",
                                                inferenceChoice === 'local' ? "bg-emerald-500 text-white" : "bg-muted text-muted-foreground group-hover:text-emerald-500"
                                            )}>
                                                <Cpu className="w-6 h-6" />
                                            </div>
                                            {inferenceChoice === 'local' && <div className="px-2 py-1 rounded-full bg-emerald-500 text-white text-[10px] font-bold uppercase tracking-wider">Selected</div>}
                                        </div>
                                        <div>
                                            <h3 className="text-lg font-bold">Local Inference</h3>
                                            <p className="text-xs text-muted-foreground mt-1">
                                                Run models directly on your device. Zero data egress, full privacy.
                                            </p>
                                        </div>
                                        <div className="flex items-center gap-2 text-[10px] font-medium text-emerald-600/80">
                                            <div className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
                                            Next: Select models to download
                                        </div>
                                    </button>

                                    <button
                                        onClick={() => setInferenceChoice('cloud')}
                                        className={cn(
                                            "relative p-6 rounded-xl border-2 text-left transition-all space-y-4 overflow-hidden group",
                                            inferenceChoice === 'cloud'
                                                ? "bg-indigo-500/5 border-indigo-500/50 shadow-lg shadow-indigo-500/10"
                                                : "bg-card border-border hover:border-indigo-500/30 hover:bg-indigo-500/5"
                                        )}
                                    >
                                        <div className="flex items-start justify-between">
                                            <div className={cn("p-3 rounded-xl transition-colors",
                                                inferenceChoice === 'cloud' ? "bg-indigo-500 text-white" : "bg-muted text-muted-foreground group-hover:text-indigo-500"
                                            )}>
                                                <Globe className="w-6 h-6" />
                                            </div>
                                            {inferenceChoice === 'cloud' && <div className="px-2 py-1 rounded-full bg-indigo-500 text-white text-[10px] font-bold uppercase tracking-wider">Selected</div>}
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
                                                        "flex items-center justify-center gap-2 transition-all shadow-sm",
                                                        "hover:translate-y-[-1px] active:translate-y-0",
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
                                        Choose models for each capability.
                                        {engineInfo && <span className="text-xs opacity-70 block mt-1">Filtered for {engineInfo.display_name} ({engineInfo.hf_tag?.toUpperCase() || 'compatible'} format).</span>}
                                    </p>
                                </div>

                                {/* Per-category sections */}
                                <div className="space-y-4">
                                    {(['llm', 'embedding', 'stt', 'diffusion'] as ModelCategory[]).map((cat) => {
                                        const filter = ONBOARDING_PIPELINE_FILTERS[cat];
                                        const topModels = categoryTopModels[cat] || [];
                                        const searchResults = categorySearchResults[cat] || [];
                                        const isSearching = categorySearching[cat] || false;
                                        const query = categorySearchQuery[cat] || '';
                                        const showSearch = categoryShowSearch[cat] || false;
                                        const selected = categorySelectedModel[cat];
                                        const enabled = categoryEnabled[cat];
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
                                                    onClick={() => setCategoryEnabled(prev => ({ ...prev, [cat]: !prev[cat] }))}
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
                                                        </div>
                                                    </div>
                                                    <div className={cn(
                                                        "w-10 h-5 rounded-full transition-colors flex items-center px-0.5",
                                                        enabled ? "bg-primary" : "bg-muted"
                                                    )}>
                                                        <div className={cn(
                                                            "w-4 h-4 rounded-full bg-white shadow-sm transition-transform",
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
                                                                            className="w-full pl-8 pr-3 py-2 text-xs bg-background border border-border/50 rounded-lg focus:outline-none focus:ring-1 focus:ring-primary/20 text-foreground placeholder:text-muted-foreground/50"
                                                                            autoFocus
                                                                        />
                                                                        {isSearching && <Loader2 className="absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground animate-spin" />}
                                                                    </div>
                                                                )}

                                                                {/* Model list */}
                                                                <div className="space-y-1.5 max-h-[200px] overflow-y-auto pr-1">
                                                                    {displayModels.length === 0 && !isSearching && (
                                                                        <div className="text-center py-4 text-xs text-muted-foreground">
                                                                            {query.trim() ? 'No models found' : <><Loader2 className="w-3 h-3 animate-spin inline mr-1" /> Loading trending models...</>}
                                                                        </div>
                                                                    )}
                                                                    {displayModels.map(model => {
                                                                        const fileInfo = hfFileInfoCache[model.id];
                                                                        return (
                                                                            <button
                                                                                key={model.id}
                                                                                onClick={() => {
                                                                                    setCategorySelectedModel(prev => ({ ...prev, [cat]: model.id }));
                                                                                    if (!hfFileInfoCache[model.id]) loadHfFileInfo(model.id);
                                                                                }}
                                                                                className={cn(
                                                                                    "w-full p-2.5 rounded-lg border text-left transition-all text-xs flex items-center gap-3",
                                                                                    selected === model.id
                                                                                        ? "border-primary bg-primary/10 shadow-sm"
                                                                                        : "border-border/50 hover:border-primary/30 bg-background/50 hover:bg-primary/5"
                                                                                )}
                                                                            >
                                                                                <div className="flex-1 min-w-0">
                                                                                    <div className="flex items-center gap-2">
                                                                                        <span className="font-bold truncate">{model.id}</span>
                                                                                        {trustedAuthors.has(model.author) && (
                                                                                            <span className="text-[8px] font-bold text-emerald-500 bg-emerald-500/10 px-1 py-0.5 rounded shrink-0">✓</span>
                                                                                        )}
                                                                                        {model.gated && (
                                                                                            <span className="text-[8px] font-bold text-amber-500 bg-amber-500/10 px-1 py-0.5 rounded shrink-0">GATED</span>
                                                                                        )}
                                                                                    </div>
                                                                                    <div className="flex items-center gap-3 mt-0.5 text-muted-foreground">
                                                                                        <span className="flex items-center gap-0.5"><ArrowDownToLine className="w-2.5 h-2.5" /> {formatDownloads(model.downloads)}</span>
                                                                                        <span className="flex items-center gap-0.5"><Heart className="w-2.5 h-2.5" /> {model.likes}</span>
                                                                                        {fileInfo && <span className="font-mono">{fileInfo.total_size_display}</span>}
                                                                                    </div>
                                                                                </div>
                                                                                {selected === model.id && <CheckCircle className="w-4 h-4 text-primary shrink-0" />}
                                                                            </button>
                                                                        );
                                                                    })}
                                                                </div>
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
                                        placeholder="hf_..."
                                        className="w-full bg-muted/50 border border-border rounded-xl px-4 py-2.5 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                    />
                                </div>

                                {/* Info box */}
                                <div className="bg-blue-500/10 border border-blue-500/20 rounded-lg p-4 text-sm text-blue-400 flex gap-3">
                                    <Info className="w-5 h-5 shrink-0" />
                                    <div>
                                        <p className="font-medium mb-1">Downloads run in background</p>
                                        <p className="opacity-90">
                                            Setup will finish immediately. You can check progress in Settings &gt; Models.
                                            <br />
                                            <button
                                                onClick={() => modelsDir && openPath(modelsDir)}
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
                                                    className="flex-1 bg-muted/50 border border-border rounded-lg px-3 py-2 text-xs font-mono focus:ring-2 focus:ring-primary/20 outline-none transition-all placeholder:text-muted-foreground/40"
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
                                <div className="w-20 h-20 bg-green-500/10 rounded-full flex items-center justify-center mx-auto mb-6">
                                    <CheckCircle className="w-10 h-10 text-green-500" />
                                </div>
                                <h2 className="text-3xl font-bold">You're All Set!</h2>
                                <p className="text-lg text-muted-foreground max-w-md mx-auto">
                                    ThinClaw Desktop is configured and ready to help. You can always change these settings later in the settings menu.
                                </p>
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
                        disabled={isLoading}
                        className="flex items-center gap-2 bg-primary text-primary-foreground hover:bg-primary/90 px-6 py-2.5 rounded-lg font-medium transition-all shadow-sm hover:shadow"
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
