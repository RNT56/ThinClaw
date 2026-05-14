import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
    MessageSquare, Database, Volume2, Mic, ImageIcon, CheckCircle2,
    Cloud, Monitor, Loader2, ChevronDown, Info, AlertTriangle, RefreshCw
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { AnimatePresence, motion } from 'framer-motion';
import { useCloudModels, type CloudModelEntry } from '../../hooks/use-cloud-models';

// ─── Types (mirroring Rust inference module) ─────────────────────────────────

interface BackendInfo {
    id: string;
    displayName: string;
    isLocal: boolean;
    modelId: string | null;
    available: boolean;
}

type Modality = 'chat' | 'embedding' | 'tts' | 'stt' | 'diffusion';

interface ModalityBackends {
    modality: Modality;
    active: BackendInfo | null;
    available: BackendInfo[];
}

// ─── Constants ───────────────────────────────────────────────────────────────

const MODALITY_META: Record<Modality, {
    label: string;
    description: string;
    icon: React.ElementType;
    color: string;
    gradient: string;
}> = {
    chat: {
        label: 'Chat & Reasoning',
        description: 'Primary LLM for conversations, Auto Mode agent, and tool use.',
        icon: MessageSquare,
        color: 'text-violet-500',
        gradient: 'from-violet-500/20 to-purple-500/10',
    },
    embedding: {
        label: 'Embeddings (RAG)',
        description: 'Converts text → vectors for semantic search and document retrieval.',
        icon: Database,
        color: 'text-cyan-500',
        gradient: 'from-cyan-500/20 to-blue-500/10',
    },
    tts: {
        label: 'Text-to-Speech',
        description: 'Generates spoken audio from text. Powers the "Read Aloud" feature.',
        icon: Volume2,
        color: 'text-amber-500',
        gradient: 'from-amber-500/20 to-orange-500/10',
    },
    stt: {
        label: 'Speech-to-Text',
        description: 'Transcribes voice input to text. Powers the microphone button.',
        icon: Mic,
        color: 'text-rose-500',
        gradient: 'from-rose-500/20 to-pink-500/10',
    },
    diffusion: {
        label: 'Image Generation',
        description: 'Creates images from text prompts. Powers Imagine Studio.',
        icon: ImageIcon,
        color: 'text-emerald-500',
        gradient: 'from-emerald-500/20 to-teal-500/10',
    },
};

const MODALITY_ORDER: Modality[] = ['chat', 'embedding', 'tts', 'stt', 'diffusion'];

// ─── TTS Voice Selector ──────────────────────────────────────────────────────

interface VoiceInfo {
    id: string;
    name: string;
    language: string | null;
    gender: string | null;
    isDefault: boolean;
}

function TtsVoiceSelector() {
    const [expanded, setExpanded] = useState(false);
    const [voices, setVoices] = useState<VoiceInfo[]>([]);
    const [loading, setLoading] = useState(false);
    const [selectedVoice, setSelectedVoice] = useState<string | null>(null);

    // Load saved voice from config
    useEffect(() => {
        (async () => {
            try {
                const config = await invoke<any>('get_user_config');
                const saved = config?.inference_models?.tts_voice;
                if (saved) setSelectedVoice(saved);
            } catch { /* ignore */ }
        })();
    }, []);

    const loadVoices = useCallback(async () => {
        if (voices.length > 0) return; // already loaded
        setLoading(true);
        try {
            const data = await invoke<VoiceInfo[]>('tts_list_voices');
            setVoices(data);
            // Auto-select the default if nothing is saved
            if (!selectedVoice) {
                const def = data.find(v => v.isDefault);
                if (def) setSelectedVoice(def.id);
            }
        } catch (e) {
            console.error('[TtsVoiceSelector] Failed to load voices:', e);
            toast.error('Failed to load TTS voices');
        } finally {
            setLoading(false);
        }
    }, [voices.length, selectedVoice]);

    const handleSelect = async (voiceId: string) => {
        setSelectedVoice(voiceId);
        try {
            const config = await invoke<any>('get_user_config');
            const models = config?.inference_models ?? {};
            models.tts_voice = voiceId;
            await invoke('update_user_config', {
                config: { ...config, inference_models: models }
            });
            toast.success(`Voice set: ${voices.find(v => v.id === voiceId)?.name ?? voiceId}`);
        } catch (e) {
            toast.error('Failed to save voice selection');
        }
    };

    const handleToggle = () => {
        const next = !expanded;
        setExpanded(next);
        if (next) loadVoices();
    };

    return (
        <div className="pt-3 border-t border-border/30 space-y-2">
            <button
                onClick={handleToggle}
                className="flex items-center gap-2 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/50 hover:text-muted-foreground transition-colors w-full"
            >
                <Volume2 className="w-3 h-3" />
                Voice Selection
                <ChevronDown className={cn("w-3 h-3 ml-auto transition-transform duration-200", expanded && "rotate-180")} />
            </button>

            <AnimatePresence>
                {expanded && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: "auto", opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        transition={{ duration: 0.2 }}
                        className="overflow-hidden"
                    >
                        {loading ? (
                            <div className="flex items-center gap-2 py-3">
                                <Loader2 className="w-3.5 h-3.5 animate-spin text-muted-foreground" />
                                <span className="text-xs text-muted-foreground">Loading voices…</span>
                            </div>
                        ) : voices.length === 0 ? (
                            <p className="text-xs text-muted-foreground/50 py-2 italic">
                                No voices available for the current backend.
                            </p>
                        ) : (
                            <div className="flex flex-wrap gap-1.5 py-1">
                                {voices.map(v => {
                                    const isSelected = v.id === selectedVoice;
                                    return (
                                        <button
                                            key={v.id}
                                            onClick={() => handleSelect(v.id)}
                                            className={cn(
                                                "text-[10px] px-2.5 py-1 rounded-lg border transition-all duration-200 font-medium",
                                                isSelected
                                                    ? "bg-primary/10 text-primary border-primary/30 ring-1 ring-primary/20 shadow-sm"
                                                    : "bg-muted/30 text-muted-foreground border-border/20 hover:bg-muted/60 hover:text-foreground"
                                            )}
                                            title={[v.name, v.gender, v.language].filter(Boolean).join(' · ')}
                                        >
                                            {v.name}
                                            {v.gender && (
                                                <span className="ml-1 opacity-50">
                                                    {v.gender === 'female' ? '♀' : v.gender === 'male' ? '♂' : ''}
                                                </span>
                                            )}
                                        </button>
                                    );
                                })}
                            </div>
                        )}
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}

// ─── Component ───────────────────────────────────────────────────────────────

function ModalitySection({
    data,
    onSwitch,
    switching,
    discoveredModels,
}: {
    data: ModalityBackends;
    onSwitch: (modality: Modality, backendId: string) => Promise<void>;
    switching: string | null;
    discoveredModels: CloudModelEntry[];
}) {
    const [open, setOpen] = useState(false);
    const meta = MODALITY_META[data.modality];
    const Icon = meta.icon;
    const active = data.active;
    const isSwitching = switching === data.modality;

    // Close dropdown on outside click
    useEffect(() => {
        if (!open) return;
        const handler = () => setOpen(false);
        window.addEventListener('click', handler);
        return () => window.removeEventListener('click', handler);
    }, [open]);

    return (
        <div className="group relative overflow-hidden rounded-2xl border border-border/50 bg-card/40 hover:bg-card/60 transition-all duration-300 shadow-sm hover:shadow-md">
            {/* Gradient accent */}
            <div className={cn(
                "absolute inset-0 bg-gradient-to-br opacity-0 group-hover:opacity-100 transition-opacity duration-500 pointer-events-none",
                meta.gradient
            )} />

            <div className="relative p-6 space-y-4">
                {/* Header */}
                <div className="flex items-start justify-between">
                    <div className="flex items-center gap-3">
                        <div className={cn(
                            "p-2.5 rounded-xl transition-all duration-300",
                            active ? "bg-gradient-to-br " + meta.gradient : "bg-muted"
                        )}>
                            <Icon className={cn("w-5 h-5", active ? meta.color : "text-muted-foreground")} />
                        </div>
                        <div>
                            <h3 className="font-bold text-base">{meta.label}</h3>
                            <p className="text-xs text-muted-foreground mt-0.5 max-w-sm">{meta.description}</p>
                        </div>
                    </div>

                    {/* Active badge */}
                    {active ? (
                        <div className="flex items-center gap-1.5 shrink-0">
                            {active.isLocal ? (
                                <span className="flex items-center gap-1.5 px-2.5 py-1 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 rounded-full text-[10px] font-bold uppercase tracking-wider border border-emerald-500/20">
                                    <Monitor className="w-3 h-3" />
                                    Local
                                </span>
                            ) : (
                                <span className="flex items-center gap-1.5 px-2.5 py-1 bg-blue-500/10 text-blue-600 dark:text-blue-400 rounded-full text-[10px] font-bold uppercase tracking-wider border border-blue-500/20">
                                    <Cloud className="w-3 h-3" />
                                    Cloud
                                </span>
                            )}
                        </div>
                    ) : (
                        <span className="flex items-center gap-1.5 px-2.5 py-1 bg-muted/50 text-muted-foreground rounded-full text-[10px] font-bold uppercase tracking-wider border border-border/50">
                            <AlertTriangle className="w-3 h-3" />
                            Not Set
                        </span>
                    )}
                </div>

                {/* Current backend + selector */}
                <div className="flex items-center gap-3">
                    <div className="flex-1 min-w-0">
                        {active ? (
                            <div className="flex items-center gap-2">
                                <CheckCircle2 className="w-4 h-4 text-emerald-500 shrink-0" />
                                <span className="font-semibold text-sm truncate">{active.displayName}</span>
                                {active.modelId && (
                                    <span className="text-[10px] bg-muted px-2 py-0.5 rounded font-mono text-muted-foreground truncate max-w-[200px]">
                                        {active.modelId}
                                    </span>
                                )}
                            </div>
                        ) : (
                            <span className="text-sm text-muted-foreground italic">
                                No backend active — select one below
                            </span>
                        )}
                    </div>

                    {/* Backend selector */}
                    <div className="relative" onClick={e => e.stopPropagation()}>
                        <button
                            onClick={() => setOpen(!open)}
                            disabled={isSwitching}
                            className={cn(
                                "flex items-center gap-2 px-4 h-9 rounded-xl text-xs font-bold uppercase tracking-wider transition-all border shadow-sm",
                                "bg-background/80 border-border/50 hover:border-primary/30 hover:bg-accent/50",
                                isSwitching && "opacity-50 cursor-wait"
                            )}
                        >
                            {isSwitching ? (
                                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                            ) : (
                                <ChevronDown className={cn("w-3.5 h-3.5 transition-transform duration-200", open && "rotate-180")} />
                            )}
                            Switch
                        </button>

                        <AnimatePresence>
                            {open && (
                                <motion.div
                                    initial={{ opacity: 0, scale: 0.95, y: -8 }}
                                    animate={{ opacity: 1, scale: 1, y: 0 }}
                                    exit={{ opacity: 0, scale: 0.95, y: -8 }}
                                    transition={{ duration: 0.15 }}
                                    className="absolute right-0 top-[calc(100%+6px)] z-50 min-w-[260px] overflow-hidden rounded-xl border border-border/50 bg-card/95 p-1.5 shadow-2xl backdrop-blur-xl"
                                >
                                    {data.available.length === 0 ? (
                                        <div className="px-3 py-4 text-xs text-muted-foreground text-center">
                                            No backends available. Add API keys in Secrets.
                                        </div>
                                    ) : (
                                        data.available.map(backend => {
                                            const isCurrent = active?.id === backend.id;
                                            return (
                                                <button
                                                    key={backend.id}
                                                    onClick={async () => {
                                                        setOpen(false);
                                                        if (!isCurrent) {
                                                            await onSwitch(data.modality, backend.id);
                                                        }
                                                    }}
                                                    disabled={!backend.available}
                                                    className={cn(
                                                        "flex items-center justify-between w-full rounded-lg px-3 py-2.5 text-sm transition-all duration-150 mb-0.5 last:mb-0",
                                                        isCurrent
                                                            ? "bg-primary/10 text-primary font-bold"
                                                            : backend.available
                                                                ? "hover:bg-muted/60 text-foreground cursor-pointer"
                                                                : "opacity-40 cursor-not-allowed"
                                                    )}
                                                >
                                                    <span className="flex items-center gap-2">
                                                        {backend.isLocal ? (
                                                            <Monitor className="w-3.5 h-3.5 text-emerald-500" />
                                                        ) : (
                                                            <Cloud className="w-3.5 h-3.5 text-blue-500" />
                                                        )}
                                                        {backend.displayName}
                                                    </span>
                                                    <span className="flex items-center gap-1.5">
                                                        {isCurrent && <CheckCircle2 className="w-3.5 h-3.5 text-emerald-500" />}
                                                        {!backend.available && (
                                                            <span className="text-[9px] text-amber-500 font-bold uppercase">No Key</span>
                                                        )}
                                                    </span>
                                                </button>
                                            );
                                        })
                                    )}
                                </motion.div>
                            )}
                        </AnimatePresence>
                    </div>
                </div>

                {/* Cost estimation for active cloud model */}
                {active && !active.isLocal && (() => {
                    // Find the active model in discovered models to get pricing
                    const activeModel = discoveredModels.find(m =>
                        active.modelId && (m.id === active.modelId || m.id.endsWith(active.modelId))
                    );
                    const pricing = activeModel?.pricing;
                    if (!pricing) return null;

                    // Build cost items based on modality
                    const items: { label: string; value: string }[] = [];

                    if (pricing.inputPerMillion != null) {
                        items.push({ label: 'Input', value: `$${pricing.inputPerMillion}/1M tok` });
                    }
                    if (pricing.outputPerMillion != null) {
                        items.push({ label: 'Output', value: `$${pricing.outputPerMillion}/1M tok` });
                    }
                    if (pricing.perImage != null) {
                        items.push({ label: 'Per image', value: `$${pricing.perImage.toFixed(3)}` });
                    }
                    if (pricing.perMinute != null) {
                        items.push({ label: 'Per minute', value: `$${pricing.perMinute.toFixed(4)}` });
                    }
                    if (pricing.per1kChars != null) {
                        items.push({ label: 'Per 1K chars', value: `$${pricing.per1kChars.toFixed(4)}` });
                    }

                    if (items.length === 0) return null;

                    return (
                        <div className="flex items-center gap-2 flex-wrap">
                            <span className="flex items-center gap-1 text-[10px] text-muted-foreground/60 font-medium">
                                <Info className="w-3 h-3" />
                                Est. cost:
                            </span>
                            {items.map((item, i) => (
                                <span
                                    key={i}
                                    className="text-[10px] px-2 py-0.5 rounded-md bg-amber-500/5 text-amber-600 dark:text-amber-400 border border-amber-500/10 font-mono"
                                >
                                    {item.label}: {item.value}
                                </span>
                            ))}
                        </div>
                    );
                })()}

                {/* Discovered models for this modality's active provider */}
                {active && !active.isLocal && discoveredModels.length > 0 && (
                    <div className="pt-3 border-t border-border/30 space-y-2">
                        <p className="text-[10px] uppercase tracking-wider font-bold text-muted-foreground/50">
                            Available Models ({discoveredModels.length})
                        </p>
                        <div className="flex flex-wrap gap-1.5">
                            {discoveredModels.slice(0, 8).map(m => (
                                <span
                                    key={m.id}
                                    className="text-[10px] px-2 py-0.5 rounded-md bg-muted/40 text-muted-foreground border border-border/20 font-mono truncate max-w-[200px]"
                                    title={`${m.displayName}${m.contextWindow ? ` · ${(m.contextWindow / 1000).toFixed(0)}K ctx` : ''}${m.pricing?.inputPerMillion ? ` · $${m.pricing.inputPerMillion}/1M in` : ''}`}
                                >
                                    {m.id}
                                </span>
                            ))}
                            {discoveredModels.length > 8 && (
                                <span className="text-[10px] px-2 py-0.5 rounded-md bg-muted/20 text-muted-foreground/60">
                                    +{discoveredModels.length - 8} more
                                </span>
                            )}
                        </div>
                    </div>
                )}

                {/* TTS Voice Selector — only for TTS when a cloud backend is active */}
                {data.modality === 'tts' && active && !active.isLocal && (
                    <TtsVoiceSelector />
                )}
            </div>
        </div>
    );
}

export function InferenceModeTab() {
    const [backends, setBackends] = useState<ModalityBackends[]>([]);
    const [loading, setLoading] = useState(true);
    const [switching, setSwitching] = useState<string | null>(null);
    const [error, setError] = useState<string | null>(null);

    // Cloud model discovery
    const { modelsByCategory } = useCloudModels();

    // Map Modality -> discovered models for that modality
    const modelsForModality = (modality: Modality): CloudModelEntry[] => {
        return modelsByCategory[modality] ?? [];
    };

    const load = useCallback(async () => {
        try {
            const data = await invoke<ModalityBackends[]>('get_inference_backends');
            setBackends(data);
            setError(null);
        } catch (e) {
            console.error('[InferenceModeTab] Failed to load backends:', e);
            setError(String(e));
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        load();
    }, [load]);

    const handleSwitch = async (modality: Modality, backendId: string) => {
        setSwitching(modality);
        try {
            await invoke('update_inference_backend', { modality, backendId });
            toast.success(`${MODALITY_META[modality].label} switched to ${backendId === 'local' ? 'Local' : backendId}`);
            // Reload to reflect changes
            await load();
        } catch (e) {
            toast.error(`Failed to switch: ${e}`);
        } finally {
            setSwitching(null);
        }
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center p-20">
                <Loader2 className="w-8 h-8 animate-spin text-primary/50" />
            </div>
        );
    }

    if (error) {
        return (
            <div className="flex flex-col items-center justify-center p-20 gap-4 text-center">
                <AlertTriangle className="w-10 h-10 text-amber-500" />
                <p className="text-sm text-muted-foreground max-w-md">
                    Failed to load inference backends. This usually means the router hasn't been initialized yet.
                </p>
                <button
                    onClick={() => { setLoading(true); load(); }}
                    className="flex items-center gap-2 px-4 py-2 rounded-xl bg-primary text-primary-foreground text-xs font-bold uppercase tracking-wider hover:bg-primary/90 transition-all"
                >
                    <RefreshCw className="w-3.5 h-3.5" />
                    Retry
                </button>
            </div>
        );
    }

    // Sort backends by our preferred order
    const sorted = MODALITY_ORDER.map(m => backends.find(b => b.modality === m)).filter(Boolean) as ModalityBackends[];

    // Summary stats
    const cloudCount = sorted.filter(m => m.active && !m.active.isLocal).length;
    const localCount = sorted.filter(m => m.active && m.active.isLocal).length;
    const unconfigured = sorted.filter(m => !m.active).length;

    return (
        <div className="space-y-8 pb-20">
            <div className="flex flex-col gap-1">
                <h2 className="text-2xl font-bold">Inference Mode</h2>
                <p className="text-muted-foreground">Configure which backend powers each AI capability — local, cloud, or hybrid.</p>
            </div>

            {/* Summary bar */}
            <div className="flex items-center gap-4 p-4 rounded-xl border border-border/50 bg-card/30">
                {localCount > 0 && (
                    <div className="flex items-center gap-1.5 text-xs font-bold text-emerald-600 dark:text-emerald-400">
                        <Monitor className="w-3.5 h-3.5" />
                        {localCount} Local
                    </div>
                )}
                {cloudCount > 0 && (
                    <div className="flex items-center gap-1.5 text-xs font-bold text-blue-600 dark:text-blue-400">
                        <Cloud className="w-3.5 h-3.5" />
                        {cloudCount} Cloud
                    </div>
                )}
                {unconfigured > 0 && (
                    <div className="flex items-center gap-1.5 text-xs font-bold text-amber-600 dark:text-amber-400">
                        <AlertTriangle className="w-3.5 h-3.5" />
                        {unconfigured} Not Set
                    </div>
                )}
                <div className="flex-1" />
                <button
                    onClick={() => { setLoading(true); load(); }}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-wider text-muted-foreground hover:text-foreground hover:bg-accent/50 transition-all"
                >
                    <RefreshCw className="w-3 h-3" />
                    Refresh
                </button>
            </div>

            {/* Modality cards */}
            <div className="grid gap-4">
                {sorted.map(data => (
                    <ModalitySection
                        key={data.modality}
                        data={data}
                        onSwitch={handleSwitch}
                        switching={switching}
                        discoveredModels={modelsForModality(data.modality)}
                    />
                ))}
            </div>

            {/* Info footer */}
            <div className="p-5 rounded-2xl border border-primary/10 bg-primary/5 flex gap-4 items-start">
                <div className="p-2.5 bg-primary/10 rounded-xl shrink-0">
                    <Info className="w-5 h-5 text-primary" />
                </div>
                <div className="space-y-1.5">
                    <p className="text-sm font-bold">How It Works</p>
                    <p className="text-xs text-muted-foreground leading-relaxed">
                        Each AI capability can use a different backend independently.
                        <span className="text-foreground font-medium"> Local</span> backends run on your hardware (private, offline).
                        <span className="text-foreground font-medium"> Cloud</span> backends call remote APIs (faster, often higher quality, requires API key).
                        Mix and match for the best experience — e.g., cloud chat with local embeddings.
                    </p>
                </div>
            </div>
        </div>
    );
}
