import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    CheckCircle, ChevronRight, Monitor, Globe, Cpu, Code, HardDrive, Info, Palette, Moon, Sun,
    Search, Heart, ArrowDownToLine, Loader2, Zap, Wrench, AlertTriangle, CheckCircle2
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { useTheme } from '../theme-provider';
import { APP_THEMES } from '../../lib/app-themes';
import { toast } from 'sonner';
import { MODEL_LIBRARY } from '../../lib/model-library';
import { useModelContext } from '../model-context';
import { openPath } from '../../lib/openclaw';
import { commands } from '../../lib/bindings';
import { invoke } from '@tauri-apps/api/core';
import { useEngineSetup } from '../../hooks/useEngineSetup';

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

type Step = 'welcome' | 'style' | 'mode' | 'engine_setup' | 'models' | 'permissions' | 'complete';
type ModelPackageType = 'none' | 'llm' | 'llm_whisper' | 'llm_diffusion' | 'full';

export function OnboardingWizard({ onComplete }: OnboardingWizardProps) {
    const [step, setStep] = useState<Step>('welcome');
    const [mode, setMode] = useState<'local' | 'remote'>('local');
    const [permissions, setPermissions] = useState({
        accessibility: false,
        screen_recording: false
    });
    const [isLoading, setIsLoading] = useState(false);
    const [selectedPackage, setSelectedPackage] = useState<ModelPackageType>('none');
    const [expandedPackage, setExpandedPackage] = useState<ModelPackageType | null>(null);
    const [selectedEmbeddingModel, setSelectedEmbeddingModel] = useState<string>('mxbai-embed-xsmall-v1');
    const [selectedDiffusionModel, setSelectedDiffusionModel] = useState<string>('flux-2-klein-4b');
    const [selectedBaseLLM, setSelectedBaseLLM] = useState<string>('ministral-3-3b-instruct');
    const [packageQuantSettings, setPackageQuantSettings] = useState<Record<string, string>>({
        'ministral-3-8b-instruct': 'Q8_0',
        'ministral-3-3b-instruct': 'Q8_0',
        'sd-3.5-medium-second-state': 'Q8_0',
        'flux-2-klein-9b-unsloth': 'Q4_K_S',
        'flux-2-klein-4b': 'Base 4B Q4_0'
    });

    const [hfToken, setHfToken] = useState<string>('');

    // ---------------------------------------------------------------------------
    // Engine setup hook + HF Hub state (for MLX/vLLM LLM selection)
    // ---------------------------------------------------------------------------
    const engineSetup = useEngineSetup();

    const [hfTopModels, setHfTopModels] = useState<HfModelCard[]>([]);
    const [hfSearchQuery, setHfSearchQuery] = useState('');
    const [hfSearchResults, setHfSearchResults] = useState<HfModelCard[]>([]);
    const [isSearchingHf, setIsSearchingHf] = useState(false);
    const [isLoadingTopModels, setIsLoadingTopModels] = useState(false);
    const [selectedHfModel, setSelectedHfModel] = useState<string | null>(null);
    const [hfFileInfoCache, setHfFileInfoCache] = useState<Record<string, ModelDownloadInfo>>({});
    const [hfShowSearch, setHfShowSearch] = useState(false);
    const hfDebounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Check if the currently selected diffusion model is gated
    const isSelectedModelGated = useMemo(() => {
        if (!selectedDiffusionModel) return false;
        const m = MODEL_LIBRARY.find(mod => mod.id === selectedDiffusionModel);
        return !!m?.gated;
    }, [selectedDiffusionModel]);

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
        startDownload,
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
    const isHfEngine = engineInfo?.id === 'mlx' || engineInfo?.id === 'vllm';
    const isOllamaEngine = engineInfo?.id === 'ollama';
    const showEngineSetupStep = engineInfo?.id === 'mlx' || engineInfo?.id === 'vllm';

    // Step list (engine_setup only for MLX/vLLM)
    const stepList = useMemo(() => {
        const s: Step[] = ['welcome', 'style', 'mode'];
        if (showEngineSetupStep) s.push('engine_setup');
        s.push('models', 'permissions', 'complete');
        return s;
    }, [showEngineSetupStep]);

    const progressPct = useMemo(() => {
        const idx = stepList.indexOf(step);
        return ((idx + 1) / stepList.length) * 100;
    }, [step, stepList]);

    // Suppress ModelProvider first-run toast during onboarding
    useEffect(() => {
        localStorage.setItem('scrappy_onboarding_in_progress', 'true');
        return () => { localStorage.removeItem('scrappy_onboarding_in_progress'); };
    }, []);

    // HF: load top 5 LLM models when entering models step (MLX/vLLM only)
    // Uses pipeline_tag: 'text-generation' to ensure only LLMs are returned
    useEffect(() => {
        if (!isHfEngine || !engineInfo || step !== 'models') return;
        if (hfTopModels.length > 0) return;
        setIsLoadingTopModels(true);
        invoke<HfModelCard[]>('discover_hf_models', {
            query: '', engine: engineInfo.id, limit: 5,
            pipelineTags: ['text-generation', 'image-text-to-text'],
        })
            .then(models => {
                setHfTopModels(models);
                if (models.length > 0 && !selectedHfModel) setSelectedHfModel(models[0].id);
            })
            .catch(err => {
                console.error('Failed to load top models:', err);
                toast.error('Failed to load recommended models from HuggingFace');
            })
            .finally(() => setIsLoadingTopModels(false));
    }, [step, isHfEngine, engineInfo]);

    // HF: debounced search (works for ALL engines — llama.cpp uses gguf tag, MLX uses mlx tag, etc.)
    useEffect(() => {
        if (!hfSearchQuery.trim() || !engineInfo) {
            setHfSearchResults([]);
            return;
        }
        if (hfDebounceTimer.current) clearTimeout(hfDebounceTimer.current);
        hfDebounceTimer.current = setTimeout(async () => {
            setIsSearchingHf(true);
            try {
                const results = await invoke<HfModelCard[]>('discover_hf_models', {
                    query: hfSearchQuery, engine: engineInfo.id, limit: 10,
                    pipelineTags: ['text-generation', 'image-text-to-text'],
                });
                setHfSearchResults(results);
                // Pre-fetch file info for all results so we can display sizes
                for (const model of results) {
                    if (!hfFileInfoCache[model.id]) {
                        invoke<ModelDownloadInfo>('get_model_files', { repoId: model.id, engine: engineInfo.id })
                            .then(info => setHfFileInfoCache(prev => ({ ...prev, [model.id]: info })))
                            .catch(() => { /* silent */ });
                    }
                }
            } catch { /* silent */ } finally { setIsSearchingHf(false); }
        }, 350);
        return () => { if (hfDebounceTimer.current) clearTimeout(hfDebounceTimer.current); };
    }, [hfSearchQuery, engineInfo]);

    // HF: load file info for selected model
    const loadHfFileInfo = useCallback(async (repoId: string) => {
        if (hfFileInfoCache[repoId] || !engineInfo) return;
        try {
            const info = await invoke<ModelDownloadInfo>('get_model_files', { repoId, engine: engineInfo.id });
            setHfFileInfoCache(prev => ({ ...prev, [repoId]: info }));
        } catch (err) { console.error('Failed to load file info:', err); }
    }, [engineInfo, hfFileInfoCache]);

    useEffect(() => {
        if (selectedHfModel && !hfFileInfoCache[selectedHfModel]) loadHfFileInfo(selectedHfModel);
    }, [selectedHfModel, loadHfFileInfo]);

    useEffect(() => {
        checkPermissions();
        const interval = setInterval(checkPermissions, 2000);
        return () => clearInterval(interval);
    }, []);

    const checkPermissions = async () => {
        try {
            const perms = await openclaw.getPermissionStatus();
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

    // Calculate package size and define models
    const packageDetails = useMemo(() => {
        const getModelInfo = (id: string) => {
            const m = MODEL_LIBRARY.find(mod => mod.id === id);
            if (!m) return null;
            const selectedQuant = packageQuantSettings[id];
            const variant = m.variants.find(v => v.name === selectedQuant) || m.variants[0];

            const parseSize = (s?: string) => {
                if (!s || s === 'Unknown') return 0;
                if (s.includes('GB')) return parseFloat(s);
                if (s.includes('MB')) return parseFloat(s) / 1024;
                return 0;
            };

            let totalModelSize = parseSize(variant.size);
            // Add components if any
            if (m.components) {
                m.components.forEach(c => {
                    totalModelSize += parseSize(c.size);
                });
            }
            if (m.mmproj) {
                totalModelSize += parseSize(m.mmproj.size);
            }

            return {
                id,
                name: m.name,
                variant,
                size: totalModelSize,
                hasVariants: m.variants.length > 1,
                variants: m.variants,
                category: m.category
            };
        };

        const getIdsForPackage = (key: string) => {
            if (isHfEngine || isOllamaEngine) {
                // LLM comes from HF Hub / Ollama, not curated library
                const extras = [selectedEmbeddingModel];
                if (key === 'llm') return extras;
                if (key === 'llm_whisper') return [...extras, 'whisper-large-v3-turbo'];
                if (key === 'llm_diffusion') return [...extras, selectedDiffusionModel];
                if (key === 'full') return [...extras, 'whisper-large-v3-turbo', selectedDiffusionModel];
                return [];
            }
            const base = [selectedBaseLLM, selectedEmbeddingModel];
            if (key === 'llm') return base;
            if (key === 'llm_whisper') return [...base, 'whisper-large-v3-turbo'];
            if (key === 'llm_diffusion') return [...base, selectedDiffusionModel];
            if (key === 'full') return [...base, 'whisper-large-v3-turbo', selectedDiffusionModel];
            return [];
        };

        const result: Record<string, { models: any[], totalSize: number }> = {};

        ['none', 'llm', 'llm_whisper', 'llm_diffusion', 'full'].forEach((key) => {
            const ids = getIdsForPackage(key);
            const models = ids.map(id => getModelInfo(id)).filter((m): m is any => m !== null);
            const totalSize = models.reduce((acc, m) => acc + (m?.size || 0), 0);
            result[key] = { models, totalSize };
        });

        return result;
    }, [packageQuantSettings, selectedEmbeddingModel, selectedDiffusionModel, selectedBaseLLM, isHfEngine, isOllamaEngine]);

    const triggerDownloads = async () => {
        if (selectedPackage === 'none') return;

        const packageInfo = packageDetails[selectedPackage];
        if (!packageInfo) return;

        // Build a set of all existing model paths for fast lookup
        const existingPaths = new Set(localModels.map((m: any) => m.path));

        for (const mInfo of packageInfo.models) {
            const m = MODEL_LIBRARY.find(mod => mod.id === mInfo.id);
            if (m) {
                // Skip curated LLM download if user chose an HF model instead
                if ((m.category === 'LLM' || !m.category) && selectedHfModel) continue;

                // Skip if model file already exists on disk
                const category = m.category || 'LLM';
                const sanitizedName = m.name.replace(/[^a-zA-Z0-9-_]/g, '_');
                const expectedPath = `${category}/${sanitizedName}/${mInfo.variant?.filename}`;
                if (existingPaths.has(expectedPath)) {
                    console.log(`Skipping download — already present: ${expectedPath}`);
                    continue;
                }

                startDownload(m as any, mInfo.variant).catch(e => console.error(`Failed to start download for ${m.id}`, e));
            }
        }
    };

    const handleFinish = async () => {
        setIsLoading(true);
        try {
            // Save HF Token first if provided
            if (hfToken && hfToken.trim().length > 0) {
                await openclaw.setHfToken(hfToken.trim());
            }

            // Clear onboarding flag
            localStorage.removeItem('scrappy_onboarding_in_progress');

            // Trigger downloads if any
            if (selectedPackage !== 'none') {
                const packageInfo = packageDetails[selectedPackage];

                // --- HF model download (any engine where user selected an HF model) ---
                if (selectedHfModel) {
                    const sanitizedRepo = selectedHfModel.replace(/\//g, '_');
                    const hfLlmDir = `LLM/${sanitizedRepo}`;
                    // Check if this HF model directory already exists (any file under it)
                    const alreadyDownloaded = localModels.some((m: any) => m.path?.startsWith(hfLlmDir));

                    if (alreadyDownloaded) {
                        console.log(`Skipping HF download — already present: ${hfLlmDir}`);
                        setModelPath(hfLlmDir);
                    } else {
                        let fileInfo = hfFileInfoCache[selectedHfModel];
                        if (!fileInfo && engineInfo) {
                            fileInfo = await invoke<ModelDownloadInfo>('get_model_files', {
                                repoId: selectedHfModel, engine: engineInfo.id,
                            });
                        }
                        if (fileInfo) {
                            const allFiles = fileInfo.files.map(f => f.filename);
                            if (fileInfo.mmproj_file) allFiles.push(fileInfo.mmproj_file.filename);
                            downloadHfFiles(selectedHfModel, allFiles).catch(e =>
                                console.error('Failed to download HF model:', e)
                            );
                            setModelPath(hfLlmDir);
                        }
                    }
                }

                // --- Curated model paths + downloads (Embedding/STT/Diffusion, or LLM for llama.cpp if no HF model selected) ---
                for (const mInfo of packageInfo.models) {
                    const m = MODEL_LIBRARY.find(mod => mod.id === mInfo.id);
                    if (m && mInfo.variant) {
                        const category = (m.category as string) || "LLM";
                        // Skip curated LLM if user selected an HF model instead
                        if (category === "LLM" && selectedHfModel) continue;
                        const sanitizedName = m.name.replace(/[^a-zA-Z0-9-_]/g, "_");
                        const mPath = `${category}/${sanitizedName}/${mInfo.variant.filename}`;

                        if (category === "LLM") setModelPath(mPath, m.template || 'auto');
                        else if (category === "Embedding") setEmbeddingModelPath(mPath);
                        else if (category === "Diffusion") setImageGenModelPath(mPath);
                        else if (category === "STT") setSttModelPath(mPath);
                    }
                }

                await triggerDownloads();
                toast.success("Background downloads started. Check Settings > Models.");
            }

            // Save setup completed status
            await openclaw.setSetupCompleted(true);
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
                                <h1 className="text-3xl font-bold tracking-tight">Welcome to Scrappy</h1>
                                <p className="text-lg text-muted-foreground max-w-md mx-auto">
                                    Your secure, private, and open-source AI desktop. Let's configure your OpenClaw experience.
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
                                    <h2 className="text-2xl font-bold">Choose Your Mode</h2>
                                    <p className="text-muted-foreground">How do you want OpenClaw to run? <span className="text-xs opacity-70 block mt-1">You can change this later in Settings.</span></p>
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
                                            <h3 className="font-semibold text-lg">Local Mode</h3>
                                            <p className="text-sm text-muted-foreground mt-1">
                                                Everything runs on your machine. Private, secure, and offline-capable.
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
                                            <Globe className="w-6 h-6" />
                                        </div>
                                        <div>
                                            <h3 className="font-semibold text-lg">Remote Mode</h3>
                                            <p className="text-sm text-muted-foreground mt-1">
                                                Connect to a powerful remote brain. Ideal for lighter devices.
                                            </p>
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
                                    <h2 className="text-2xl font-bold">Select Starting Models</h2>
                                    <p className="text-muted-foreground">Download essential models to get started immediately.</p>
                                </div>

                                <div className="grid gap-3">
                                    {[
                                        { id: 'none', label: 'No Base Package', totalSize: 0, desc: 'Download models manually later.' },
                                        { id: 'llm', label: 'LLM Base', totalSize: packageDetails.llm.totalSize, desc: 'Core text capabilities + Embeddings.' },
                                        { id: 'llm_whisper', label: 'LLM + Whisper', totalSize: packageDetails.llm_whisper.totalSize, desc: `Text + Speech recognition + Embeddings.${hasSttInstalled ? ' (Whisper installed)' : ''}` },
                                        { id: 'llm_diffusion', label: 'LLM + Diffusion', totalSize: packageDetails.llm_diffusion.totalSize, desc: 'Text + Image generation + Embeddings.' },
                                        { id: 'full', label: 'Full Suite', totalSize: packageDetails.full.totalSize, desc: `All capabilities (LLM, RAG, Image, Audio).${hasSttInstalled ? ' (Whisper installed)' : ''}` }
                                    ].map((opt) => (
                                        <div key={opt.id} className="space-y-2">
                                            <button
                                                onClick={() => setSelectedPackage(opt.id as ModelPackageType)}
                                                className={cn(
                                                    "w-full p-4 rounded-xl border-2 text-left transition-all flex items-center justify-between",
                                                    selectedPackage === opt.id
                                                        ? "border-primary bg-primary/5 shadow-md"
                                                        : "border-border hover:border-primary/50 bg-card"
                                                )}
                                            >
                                                <div className="flex-1">
                                                    <div className="flex items-center justify-between">
                                                        <h3 className="font-semibold">{opt.label}</h3>
                                                        {opt.id !== 'none' && (
                                                            <button
                                                                onClick={(e) => {
                                                                    e.stopPropagation();
                                                                    setExpandedPackage(expandedPackage === opt.id ? null : opt.id as ModelPackageType);
                                                                }}
                                                                className="text-[10px] uppercase tracking-wider font-bold text-primary/70 hover:text-primary transition-colors"
                                                            >
                                                                {expandedPackage === opt.id ? "Close Details" : "Details"}
                                                            </button>
                                                        )}
                                                    </div>
                                                    <p className="text-sm text-muted-foreground">{opt.desc}</p>
                                                </div>
                                                {opt.totalSize > 0 && (
                                                    <div className="text-right ml-4">
                                                        <span className="text-sm font-medium bg-muted px-2 py-1 rounded whitespace-nowrap">
                                                            ~{opt.totalSize.toFixed(1)} GB
                                                        </span>
                                                    </div>
                                                )}
                                            </button>

                                            <AnimatePresence>
                                                {expandedPackage === opt.id && opt.id !== 'none' && (
                                                    <motion.div
                                                        initial={{ height: 0, opacity: 0 }}
                                                        animate={{ height: "auto", opacity: 1 }}
                                                        exit={{ height: 0, opacity: 0 }}
                                                        className="overflow-hidden"
                                                    >
                                                        <div className="bg-muted/30 border border-border rounded-lg p-4 space-y-3">
                                                            <div className="space-y-4">
                                                                {/* Category Choice: Base LLM — engine-conditional */}
                                                                {!isOllamaEngine && (
                                                                    <div className="space-y-2">
                                                                        <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1 flex items-center gap-2">
                                                                            Base LLM {engineInfo ? `(${engineInfo.display_name})` : ''}
                                                                            {hasLlmInstalled && <span className="text-[9px] font-semibold text-emerald-500 bg-emerald-500/10 px-1.5 py-0.5 rounded-full">Installed</span>}
                                                                        </h5>

                                                                        {/* llama.cpp: curated GGUF picker */}
                                                                        {!isHfEngine && (
                                                                            <div className="grid grid-cols-2 gap-2">
                                                                                {[
                                                                                    { id: 'ministral-3-3b-instruct', label: 'Ministral 3B', desc: 'Fast & Low RAM' },
                                                                                    { id: 'ministral-3-8b-instruct', label: 'Ministral 8B', desc: 'Smart & Balanced' }
                                                                                ].map(choice => (
                                                                                    <button
                                                                                        key={choice.id}
                                                                                        onClick={() => setSelectedBaseLLM(choice.id)}
                                                                                        className={cn(
                                                                                            "p-2 rounded-lg border text-left transition-all",
                                                                                            selectedBaseLLM === choice.id
                                                                                                ? "border-primary bg-primary/10"
                                                                                                : "border-border bg-background/50 hover:border-primary/50"
                                                                                        )}
                                                                                    >
                                                                                        <div className="text-xs font-bold">{choice.label}</div>
                                                                                        <div className="text-[10px] text-muted-foreground">{choice.desc}</div>
                                                                                    </button>
                                                                                ))}
                                                                            </div>
                                                                        )}

                                                                        {/* llama.cpp/Ollama: also allow HF Hub search */}
                                                                        {!isHfEngine && (
                                                                            <div className="mt-2">
                                                                                {!hfShowSearch ? (
                                                                                    <button onClick={() => setHfShowSearch(true)}
                                                                                        className="w-full py-1.5 text-[10px] text-muted-foreground hover:text-primary transition-colors flex items-center justify-center gap-1.5 border border-dashed border-border/50 rounded-lg hover:border-primary/30">
                                                                                        <Search className="w-3 h-3" /> Or search HuggingFace for {engineInfo?.hf_tag?.toUpperCase() ?? 'GGUF'} models...
                                                                                    </button>
                                                                                ) : (
                                                                                    <div className="space-y-2">
                                                                                        <div className="relative">
                                                                                            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                                                                                            <input type="text" placeholder={`Search ${engineInfo?.hf_tag?.toUpperCase() ?? 'GGUF'} models...`}
                                                                                                value={hfSearchQuery} onChange={(e) => setHfSearchQuery(e.target.value)}
                                                                                                className="w-full pl-8 pr-3 py-2 text-xs bg-background border border-border/50 rounded-lg focus:outline-none focus:ring-1 focus:ring-primary/20 text-foreground placeholder:text-muted-foreground/50" />
                                                                                            {isSearchingHf && <Loader2 className="absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground animate-spin" />}
                                                                                        </div>
                                                                                        {hfSearchResults.length > 0 && (
                                                                                            <div className="space-y-1 max-h-[120px] overflow-y-auto pr-1">
                                                                                                {hfSearchResults.map(model => (
                                                                                                    <button key={model.id} onClick={() => setSelectedHfModel(model.id)}
                                                                                                        className={cn("w-full p-2 rounded-lg border text-left transition-all text-xs",
                                                                                                            selectedHfModel === model.id ? "border-primary bg-primary/10" : "border-border bg-background/50 hover:border-primary/50")}>
                                                                                                        <div className="font-bold truncate">{model.name}</div>
                                                                                                        <div className="text-[10px] text-muted-foreground flex items-center gap-2 mt-0.5">
                                                                                                            <span className={trustedAuthors.has(model.author) ? 'text-green-500 font-medium' : ''}>{model.author}{trustedAuthors.has(model.author) && ' ✓'}</span>
                                                                                                            <span className="flex items-center gap-0.5"><ArrowDownToLine className="w-2.5 h-2.5" />{formatDownloads(model.downloads)}</span>
                                                                                                            {hfFileInfoCache[model.id] && <span className="font-mono">{hfFileInfoCache[model.id].total_size_display}</span>}
                                                                                                        </div>
                                                                                                    </button>
                                                                                                ))}
                                                                                            </div>
                                                                                        )}
                                                                                        {selectedHfModel && hfFileInfoCache[selectedHfModel] && (
                                                                                            <div className="text-[10px] text-muted-foreground bg-primary/5 border border-primary/10 rounded-lg px-2 py-1.5">
                                                                                                Selected: <span className="font-semibold text-foreground">{selectedHfModel}</span>
                                                                                                <span className="ml-2 font-mono">{hfFileInfoCache[selectedHfModel].total_size_display}</span>
                                                                                            </div>
                                                                                        )}
                                                                                    </div>
                                                                                )}
                                                                            </div>
                                                                        )}

                                                                        {/* MLX/vLLM: HF Hub model picker */}
                                                                        {isHfEngine && (
                                                                            <div className="space-y-3">
                                                                                <div className="flex items-center gap-2 text-[10px] text-muted-foreground bg-muted/30 px-2 py-1.5 rounded-lg border border-border/30">
                                                                                    <Info className="w-3 h-3 text-primary/60 shrink-0" />
                                                                                    <span>Showing <span className="font-semibold text-foreground">{engineInfo?.hf_tag?.toUpperCase()}</span> models from HuggingFace.
                                                                                        {engineInfo?.id === 'mlx' && <> Prefer <span className="font-semibold text-foreground">mlx-community</span> repos for verified compatibility.</>}
                                                                                    </span>
                                                                                </div>

                                                                                {isLoadingTopModels ? (
                                                                                    <div className="flex items-center justify-center py-4 text-xs text-muted-foreground gap-2">
                                                                                        <Loader2 className="w-3.5 h-3.5 animate-spin" /> Loading top models...
                                                                                    </div>
                                                                                ) : (
                                                                                    <div className="space-y-1.5">
                                                                                        {hfTopModels.map(model => {
                                                                                            const isSelected = selectedHfModel === model.id;
                                                                                            const fi = hfFileInfoCache[model.id];
                                                                                            return (
                                                                                                <button key={model.id} onClick={() => setSelectedHfModel(model.id)}
                                                                                                    className={cn("w-full p-2.5 rounded-lg border text-left transition-all",
                                                                                                        isSelected ? "border-primary bg-primary/10 ring-1 ring-primary/20" : "border-border bg-background/50 hover:border-primary/50")}>
                                                                                                    <div className="flex items-center justify-between">
                                                                                                        <div className="min-w-0">
                                                                                                            <div className="text-xs font-bold truncate">{model.name}</div>
                                                                                                            <div className="text-[10px] text-muted-foreground flex items-center gap-2 mt-0.5">
                                                                                                                <span className={trustedAuthors.has(model.author) ? 'text-green-500 font-medium' : ''}>{model.author}{trustedAuthors.has(model.author) && ' ✓'}</span>
                                                                                                                <span className="flex items-center gap-0.5"><ArrowDownToLine className="w-2.5 h-2.5" />{formatDownloads(model.downloads)}</span>
                                                                                                                <span className="flex items-center gap-0.5"><Heart className="w-2.5 h-2.5" />{formatDownloads(model.likes)}</span>
                                                                                                                {fi && <span className="font-mono">{fi.total_size_display}</span>}
                                                                                                            </div>
                                                                                                        </div>
                                                                                                        {isSelected && <CheckCircle2 className="w-4 h-4 text-primary shrink-0" />}
                                                                                                    </div>
                                                                                                </button>
                                                                                            );
                                                                                        })}
                                                                                    </div>
                                                                                )}

                                                                                {!hfShowSearch ? (
                                                                                    <button onClick={() => setHfShowSearch(true)}
                                                                                        className="w-full py-2 text-[11px] text-muted-foreground hover:text-primary transition-colors flex items-center justify-center gap-1.5">
                                                                                        <Search className="w-3 h-3" /> Search for more models...
                                                                                    </button>
                                                                                ) : (
                                                                                    <div className="space-y-2">
                                                                                        <div className="relative">
                                                                                            <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
                                                                                            <input type="text" placeholder={`Search ${engineInfo?.hf_tag?.toUpperCase() ?? ''} LLMs on HuggingFace...`}
                                                                                                value={hfSearchQuery} onChange={(e) => setHfSearchQuery(e.target.value)}
                                                                                                className="w-full pl-8 pr-3 py-2 text-xs bg-background border border-border/50 rounded-lg focus:outline-none focus:ring-1 focus:ring-primary/20 text-foreground placeholder:text-muted-foreground/50" />
                                                                                            {isSearchingHf && <Loader2 className="absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground animate-spin" />}
                                                                                        </div>
                                                                                        {hfSearchResults.length > 0 && (
                                                                                            <div className="space-y-1 max-h-[150px] overflow-y-auto pr-1">
                                                                                                {hfSearchResults.map(model => (
                                                                                                    <button key={model.id} onClick={() => setSelectedHfModel(model.id)}
                                                                                                        className={cn("w-full p-2 rounded-lg border text-left transition-all text-xs",
                                                                                                            selectedHfModel === model.id ? "border-primary bg-primary/10" : "border-border bg-background/50 hover:border-primary/50")}>
                                                                                                        <div className="font-bold truncate">{model.name}</div>
                                                                                                        <div className="text-[10px] text-muted-foreground flex items-center gap-2 mt-0.5">
                                                                                                            <span className={trustedAuthors.has(model.author) ? 'text-green-500 font-medium' : ''}>{model.author}{trustedAuthors.has(model.author) && ' ✓'}</span>
                                                                                                            <span className="flex items-center gap-0.5"><ArrowDownToLine className="w-2.5 h-2.5" />{formatDownloads(model.downloads)}</span>
                                                                                                            {hfFileInfoCache[model.id] && <span className="font-mono">{hfFileInfoCache[model.id].total_size_display}</span>}
                                                                                                        </div>
                                                                                                    </button>
                                                                                                ))}
                                                                                            </div>
                                                                                        )}
                                                                                    </div>
                                                                                )}
                                                                            </div>
                                                                        )}
                                                                    </div>
                                                                )}

                                                                {isOllamaEngine && (
                                                                    <div className="space-y-2">
                                                                        <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1">Base LLM (Ollama)</h5>
                                                                        <div className="text-xs text-muted-foreground bg-muted/30 border border-border/30 rounded-lg p-3 space-y-2">
                                                                            <p>Ollama manages LLM models via its own registry. Use the terminal:</p>
                                                                            <code className="block bg-background/80 rounded px-2 py-1 font-mono text-[10px]">ollama pull gemma3:4b</code>
                                                                        </div>
                                                                    </div>
                                                                )}

                                                                {/* Category Choice: Embedding (if in package) */}
                                                                {packageDetails[opt.id].models.some(m => m.category === 'Embedding') && (
                                                                    <div className="space-y-2">
                                                                        <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1 flex items-center gap-2">
                                                                            Embedding Engine
                                                                            {hasEmbeddingInstalled && <span className="text-[9px] font-semibold text-emerald-500 bg-emerald-500/10 px-1.5 py-0.5 rounded-full">Installed</span>}
                                                                        </h5>
                                                                        <div className="grid grid-cols-2 gap-2">
                                                                            {[
                                                                                { id: 'mxbai-embed-xsmall-v1', label: 'MxBai (X-Small)', desc: 'Fast & Lightweight' },
                                                                                { id: 'mxbai-embed-large-v1', label: 'MxBai (Large)', desc: 'High Accuracy' }
                                                                            ].map(choice => (
                                                                                <button
                                                                                    key={choice.id}
                                                                                    onClick={() => setSelectedEmbeddingModel(choice.id)}
                                                                                    className={cn(
                                                                                        "p-2 rounded-lg border text-left transition-all",
                                                                                        selectedEmbeddingModel === choice.id
                                                                                            ? "border-primary bg-primary/10"
                                                                                            : "border-border bg-background/50 hover:border-primary/50"
                                                                                    )}
                                                                                >
                                                                                    <div className="text-xs font-bold">{choice.label}</div>
                                                                                    <div className="text-[10px] text-muted-foreground">{choice.desc}</div>
                                                                                </button>
                                                                            ))}
                                                                        </div>
                                                                    </div>
                                                                )}

                                                                {/* Category Choice: Diffusion (if in package) */}
                                                                {packageDetails[opt.id].models.some(m => m.category === 'Diffusion') && (
                                                                    <div className="space-y-2">
                                                                        <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1 flex items-center gap-2">
                                                                            Image Generation Engine
                                                                            {hasDiffusionInstalled && <span className="text-[9px] font-semibold text-emerald-500 bg-emerald-500/10 px-1.5 py-0.5 rounded-full">Installed</span>}
                                                                        </h5>
                                                                        <div className="grid grid-cols-2 gap-2">
                                                                            {[
                                                                                { id: 'flux-klein', label: 'Flux Klein', desc: 'Fast & High Quality' },
                                                                                { id: 'sd-3.5-medium-second-state', label: 'SD 3.5 Medium', desc: 'Versatile GGUF' }
                                                                            ].map(choice => (
                                                                                <button
                                                                                    key={choice.id}
                                                                                    onClick={() => {
                                                                                        if (choice.id === 'flux-klein') {
                                                                                            // Don't change ID if it's already one of the klein variants
                                                                                            if (!selectedDiffusionModel.startsWith('flux-2-klein')) {
                                                                                                setSelectedDiffusionModel('flux-2-klein-9b-unsloth');
                                                                                            }
                                                                                        } else {
                                                                                            setSelectedDiffusionModel(choice.id);
                                                                                        }
                                                                                    }}
                                                                                    className={cn(
                                                                                        "p-2 rounded-lg border text-left transition-all",
                                                                                        (choice.id === 'flux-klein' ? selectedDiffusionModel.startsWith('flux-2-klein') : selectedDiffusionModel === choice.id)
                                                                                            ? "border-primary bg-primary/10"
                                                                                            : "border-border bg-background/50 hover:border-primary/50"
                                                                                    )}
                                                                                >
                                                                                    <div className="text-xs font-bold">{choice.label}</div>
                                                                                    <div className="text-[10px] text-muted-foreground">{choice.desc}</div>
                                                                                </button>
                                                                            ))}
                                                                        </div>

                                                                        {/* Sub-selector for Flux Klein variants */}
                                                                        {selectedDiffusionModel.startsWith('flux-2-klein') && (
                                                                            <div className="flex gap-2 p-1 bg-background/50 rounded-lg border border-border">
                                                                                {[
                                                                                    { id: 'flux-2-klein-4b', label: '4B (Ultra-Light)' },
                                                                                    { id: 'flux-2-klein-9b-unsloth', label: '9B (High Quality)' }
                                                                                ].map(v => (
                                                                                    <button
                                                                                        key={v.id}
                                                                                        onClick={() => setSelectedDiffusionModel(v.id)}
                                                                                        className={cn(
                                                                                            "flex-1 py-1 text-[10px] font-bold rounded-md transition-all",
                                                                                            selectedDiffusionModel === v.id
                                                                                                ? "bg-primary text-primary-foreground shadow-sm"
                                                                                                : "text-muted-foreground hover:text-foreground"
                                                                                        )}
                                                                                    >
                                                                                        {v.label}
                                                                                    </button>
                                                                                ))}
                                                                            </div>
                                                                        )}
                                                                        {isSelectedModelGated && (
                                                                            <div className="mt-2 p-3 bg-yellow-500/10 border border-yellow-500/20 rounded-lg space-y-2">
                                                                                <div className="flex items-center gap-2 text-yellow-500">
                                                                                    <Info className="w-4 h-4" />
                                                                                    <span className="text-xs font-medium">Hugging Face Token Required</span>
                                                                                </div>
                                                                                <p className="text-[10px] text-muted-foreground">
                                                                                    The selected Flux model requires a Hugging Face token to download.
                                                                                    <button
                                                                                        onClick={() => commands.openUrl("https://huggingface.co/settings/tokens")}
                                                                                        className="text-primary hover:underline ml-1 inline-flex items-center gap-0.5"
                                                                                    >
                                                                                        Get Token <Globe className="w-3 h-3" />
                                                                                    </button>
                                                                                </p>
                                                                                <input
                                                                                    type="password"
                                                                                    placeholder="hf_..."
                                                                                    value={hfToken}
                                                                                    onChange={(e) => setHfToken(e.target.value)}
                                                                                    className="w-full bg-background border border-border rounded px-2 py-1.5 text-xs focus:ring-1 focus:ring-primary outline-none"
                                                                                />
                                                                            </div>
                                                                        )}
                                                                    </div>
                                                                )}

                                                                <div className="space-y-2">
                                                                    <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1">Included Models & Precision</h5>
                                                                    <div className="space-y-2">
                                                                        {packageDetails[opt.id].models.map((m: any) => (
                                                                            <div key={m.id} className="flex flex-col gap-1 p-2 bg-background/50 rounded-md border border-border/50">
                                                                                <div className="flex items-center justify-between">
                                                                                    <span className="text-sm font-medium">{m.name}</span>
                                                                                    <span className="text-[10px] font-mono text-muted-foreground">{m.variant.size}</span>
                                                                                </div>
                                                                                {m.hasVariants && (
                                                                                    <div className="flex gap-1.5 overflow-x-auto pb-1 no-scrollbar">
                                                                                        {m.variants.map((v: any) => (
                                                                                            <button
                                                                                                key={v.name}
                                                                                                onClick={() => setPackageQuantSettings(prev => ({ ...prev, [m.id]: v.name }))}
                                                                                                className={cn(
                                                                                                    "text-[10px] px-2 py-0.5 rounded border transition-all whitespace-nowrap",
                                                                                                    packageQuantSettings[m.id] === v.name
                                                                                                        ? "bg-primary text-primary-foreground border-primary"
                                                                                                        : "border-border hover:bg-muted text-muted-foreground"
                                                                                                )}
                                                                                            >
                                                                                                {v.name}
                                                                                            </button>
                                                                                        ))}
                                                                                    </div>
                                                                                )}
                                                                            </div>
                                                                        ))}
                                                                    </div>
                                                                </div>
                                                            </div>
                                                        </div>
                                                    </motion.div>
                                                )}
                                            </AnimatePresence>
                                        </div>
                                    ))}
                                </div>

                                {selectedPackage !== 'none' && (
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
                                                {" "}- {engineInfo?.id === 'mlx'
                                                    ? 'Place MLX model directories here (safetensors format).'
                                                    : engineInfo?.id === 'vllm'
                                                        ? 'Place vLLM-compatible model directories here (AWQ/safetensors).'
                                                        : engineInfo?.id === 'ollama'
                                                            ? 'Ollama manages models via its own registry (ollama pull).'
                                                            : 'Drag and drop your own GGUF models here.'}
                                            </p>
                                        </div>
                                    </div>
                                )}
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
                                    <p className="text-muted-foreground">OpenClaw needs access to interact with your system. <span className="text-xs opacity-70 block mt-1">These settings can be managed later.</span></p>
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
                                                    onClick={() => openclaw.openPermissionSettings('accessibility')}
                                                    className="text-xs text-muted-foreground hover:text-foreground underline underline-offset-2 transition-colors"
                                                >
                                                    Manage
                                                </button>
                                            </div>
                                        ) : (
                                            <button
                                                onClick={async () => {
                                                    const updated = await openclaw.requestPermission('accessibility');
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
                                                    onClick={() => openclaw.openPermissionSettings('screen_recording')}
                                                    className="text-xs text-muted-foreground hover:text-foreground underline underline-offset-2 transition-colors"
                                                >
                                                    Manage
                                                </button>
                                            </div>
                                        ) : (
                                            <button
                                                onClick={async () => {
                                                    const updated = await openclaw.requestPermission('screen_recording');
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
                                    Scrappy is configured and ready to help. You can always change these settings later in the settings menu.
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
