import { useState, useEffect, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    CheckCircle, ChevronRight, Monitor, Globe, Cpu, Code, HardDrive, Info
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as clawdbot from '../../lib/clawdbot';
import { toast } from 'sonner';
// Use ModelDefinition from lib to avoid interface conflicts if re-exported differently in model-context
import { MODEL_LIBRARY } from '../../lib/model-library';
import { useModelContext } from '../model-context';
import { openPath } from '../../lib/clawdbot';
import { commands } from '../../lib/bindings';

interface OnboardingWizardProps {
    onComplete: () => void;
}

type Step = 'welcome' | 'mode' | 'models' | 'permissions' | 'complete';
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

    // Check if the currently selected diffusion model is gated
    const isSelectedModelGated = useMemo(() => {
        if (!selectedDiffusionModel) return false;
        const m = MODEL_LIBRARY.find(mod => mod.id === selectedDiffusionModel);
        return !!m?.gated;
    }, [selectedDiffusionModel]);

    // Access Model Context to trigger downloads and set paths
    const {
        startDownload,
        modelsDir,
        setModelPath,
        setImageGenModelPath,
        setEmbeddingModelPath,
        setSttModelPath
    } = useModelContext();

    useEffect(() => {
        checkPermissions();
        const interval = setInterval(checkPermissions, 2000);
        return () => clearInterval(interval);
    }, []);

    const checkPermissions = async () => {
        try {
            const perms = await clawdbot.getPermissionStatus();
            setPermissions(perms);
        } catch (e) {
            console.error("Failed to check permissions", e);
        }
    };

    const handleNext = () => {
        if (step === 'welcome') setStep('mode');
        else if (step === 'mode') setStep('models');
        else if (step === 'models') setStep('permissions');
        else if (step === 'permissions') setStep('complete');
        else if (step === 'complete') handleFinish();
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
    }, [packageQuantSettings, selectedEmbeddingModel, selectedDiffusionModel, selectedBaseLLM]);

    const triggerDownloads = async () => {
        if (selectedPackage === 'none') return;

        const packageInfo = packageDetails[selectedPackage];
        if (!packageInfo) return;

        for (const mInfo of packageInfo.models) {
            const m = MODEL_LIBRARY.find(mod => mod.id === mInfo.id);
            if (m) {
                startDownload(m as any, mInfo.variant).catch(e => console.error(`Failed to start download for ${m.id}`, e));
            }
        }
    };

    const handleFinish = async () => {
        setIsLoading(true);
        try {
            // Save HF Token first if provided
            if (hfToken && hfToken.trim().length > 0) {
                await commands.setHfToken(hfToken.trim());
            }

            // Trigger downloads if any
            if (selectedPackage !== 'none') {
                const packageInfo = packageDetails[selectedPackage];

                // Set active model paths so they're ready to use as soon as they download
                for (const mInfo of packageInfo.models) {
                    const m = MODEL_LIBRARY.find(mod => mod.id === mInfo.id);
                    if (m && mInfo.variant) {
                        const category = (m.category as string) || "LLM";
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
            await clawdbot.setSetupCompleted(true);
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
                        animate={{
                            width: step === 'welcome' ? "20%" :
                                step === 'mode' ? "40%" :
                                    step === 'models' ? "60%" :
                                        step === 'permissions' ? "80%" : "100%"
                        }}
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
                                        { id: 'llm_whisper', label: 'LLM + Whisper', totalSize: packageDetails.llm_whisper.totalSize, desc: 'Text + Speech recognition + Embeddings.' },
                                        { id: 'llm_diffusion', label: 'LLM + Diffusion', totalSize: packageDetails.llm_diffusion.totalSize, desc: 'Text + Image generation + Embeddings.' },
                                        { id: 'full', label: 'Full Suite', totalSize: packageDetails.full.totalSize, desc: 'All capabilities (LLM, RAG, Image, Audio).' }
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
                                                                {/* Category Choice: Base LLM */}
                                                                <div className="space-y-2">
                                                                    <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1">Base LLM Engine</h5>
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
                                                                </div>

                                                                {/* Category Choice: Embedding (if in package) */}
                                                                {packageDetails[opt.id].models.some(m => m.category === 'Embedding') && (
                                                                    <div className="space-y-2">
                                                                        <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1">Embedding Engine</h5>
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
                                                                        <h5 className="text-[10px] font-bold uppercase tracking-wider text-muted-foreground/70 px-1">Image Generation Engine</h5>
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
                                                {" "}- Drag and drop your own GGUF models here.
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
                                            <span className="flex items-center gap-1.5 text-green-500 text-sm font-medium bg-green-500/10 px-3 py-1 rounded-full">
                                                <CheckCircle className="w-4 h-4" /> Granted
                                            </span>
                                        ) : (
                                            <button
                                                onClick={() => clawdbot.requestPermission('accessibility')}
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
                                            <span className="flex items-center gap-1.5 text-green-500 text-sm font-medium bg-green-500/10 px-3 py-1 rounded-full">
                                                <CheckCircle className="w-4 h-4" /> Granted
                                            </span>
                                        ) : (
                                            <button
                                                onClick={() => clawdbot.requestPermission('screen_recording')}
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
                                if (step === 'mode') setStep('welcome');
                                else if (step === 'models') setStep('mode');
                                else if (step === 'permissions') setStep('models');
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
