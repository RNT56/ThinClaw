
import { useState, useEffect } from 'react';
import { commands } from '../../lib/bindings';
import { Bot, Zap, ShieldCheck, ShieldAlert, CheckCircle, Info, KeyRound } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { useConfig } from '../../hooks/use-config';

export function ChatProviderTab() {
    const { config, updateConfig } = useConfig();
    const [status, setStatus] = useState<any>(null);
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        loadAll();
    }, []);

    const loadAll = async () => {
        setLoading(true);
        try {
            const s = await commands.openclawGetStatus();
            if (s.status === 'ok') setStatus(s.data);
        } catch (e) {
            console.error(e);
            toast.error("Failed to load settings");
        } finally {
            setLoading(false);
        }
    };

    const handleSelect = async (providerId: string | null) => {
        if (!config) return;
        try {
            const newConfig = { ...config, selected_chat_provider: providerId };
            await updateConfig(newConfig);
            toast.success(`${providerId === null ? 'Local' : providerId.charAt(0).toUpperCase() + providerId.slice(1)} selected for Chat & Auto Mode`);
        } catch (e) {
            toast.error("Failed to update provider");
        }
    };

    if (loading || !config) {
        return <div className="flex items-center justify-center p-20 animate-pulse">Loading...</div>;
    }

    const providers = [
        {
            id: null,
            name: 'Local Neural Link',
            description: 'Run models directly on your hardware. Private & Offline.',
            icon: Zap,
            color: 'emerald',
            configured: true, // Local is always "configured" if they have models
            status: 'Ready'
        },
        {
            id: 'anthropic',
            name: 'Anthropic Claude',
            description: 'Claude 4.5 Sonnet & Opus. Industry-leading intelligence.',
            icon: Bot,
            color: 'indigo',
            configured: !!(status?.has_anthropic_key || status?.hasAnthropicKey),
            status: !!(status?.has_anthropic_key || status?.hasAnthropicKey) ? 'Configured' : 'Missing Token'
        },
        {
            id: 'openai',
            name: 'OpenAI GPT',
            description: 'GPT-5.2, o-series and CodeX models. Advanced reasoning & logic.',
            icon: Bot,
            color: 'blue',
            configured: !!(status?.has_openai_key || status?.hasOpenaiKey),
            status: !!(status?.has_openai_key || status?.hasOpenaiKey) ? 'Configured' : 'Missing Token'
        },
        {
            id: 'openrouter',
            name: 'OpenRouter',
            description: 'Unified API for deepseek, llama, and hundreds more.',
            icon: Bot,
            color: 'purple',
            configured: !!(status?.has_openrouter_key || status?.hasOpenrouterKey),
            status: !!(status?.has_openrouter_key || status?.hasOpenrouterKey) ? 'Configured' : 'Missing Token'
        },
        {
            id: 'gemini',
            name: 'Google Gemini',
            description: 'Google\'s most capable models. Advanced multimodal reasoning.',
            icon: Bot,
            color: 'cyan',
            configured: !!(status?.has_gemini_key || status?.hasGeminiKey),
            status: !!(status?.has_gemini_key || status?.hasGeminiKey) ? 'Configured' : 'Missing Token'
        },
        {
            id: 'groq',
            name: 'Groq Cloud',
            description: 'LPU-powered inference for blazing-fast responses.',
            icon: Bot,
            color: 'orange',
            configured: !!(status?.has_groq_key || status?.hasGroqKey),
            status: !!(status?.has_groq_key || status?.hasGroqKey) ? 'Configured' : 'Missing Token'
        }
    ];

    const currentProvider = config.selected_chat_provider;

    const getColorClasses = (color: string, isSelected: boolean) => {
        const colors: Record<string, { bg: string, border: string, text: string, iconBg: string, shadow: string }> = {
            emerald: {
                bg: 'bg-emerald-500/10',
                border: 'border-emerald-500/40',
                text: 'text-emerald-500',
                iconBg: 'bg-emerald-500/20',
                shadow: 'shadow-emerald-500/5'
            },
            indigo: {
                bg: 'bg-indigo-500/10',
                border: 'border-indigo-500/40',
                text: 'text-indigo-500',
                iconBg: 'bg-indigo-500/20',
                shadow: 'shadow-indigo-500/5'
            },
            blue: {
                bg: 'bg-blue-500/10',
                border: 'border-blue-500/40',
                text: 'text-blue-500',
                iconBg: 'bg-blue-500/20',
                shadow: 'shadow-blue-500/5'
            },
            purple: {
                bg: 'bg-purple-500/10',
                border: 'border-purple-500/40',
                text: 'text-purple-500',
                iconBg: 'bg-purple-500/20',
                shadow: 'shadow-purple-500/5'
            }
        };

        const c = colors[color] || colors.emerald;
        return isSelected ? `${c.bg} ${c.border} ${c.shadow} ${c.text}` : "";
    };

    const getIconClasses = (color: string, isSelected: boolean) => {
        const colors: Record<string, string> = {
            emerald: 'bg-emerald-500/20 text-emerald-500',
            indigo: 'bg-indigo-500/20 text-indigo-500',
            blue: 'bg-blue-500/20 text-blue-500',
            purple: 'bg-purple-500/20 text-purple-500'
        };
        return isSelected ? colors[color] : 'bg-muted text-muted-foreground group-hover:text-foreground';
    };

    return (
        <div className="space-y-8 pb-20">
            <div className="flex flex-col gap-1">
                <h2 className="text-2xl font-bold">Chat Provider</h2>
                <p className="text-muted-foreground">Select the primary intelligence engine for normal Chat and Auto Mode (Rig Agent).</p>
            </div>

            <div className="grid gap-4">
                {providers.map((p) => {
                    const Icon = p.icon;
                    const isSelected = currentProvider === p.id;
                    const canSelect = p.configured;

                    return (
                        <button
                            key={p.id ?? 'local'}
                            onClick={() => {
                                if (canSelect) handleSelect(p.id);
                                else {
                                    toast.info(`Please configure ${p.name} in the Secrets tab first.`);
                                    window.dispatchEvent(new CustomEvent('open-settings', { detail: 'secrets' }));
                                }
                            }}
                            className={cn(
                                "p-6 rounded-2xl border transition-all text-left flex items-start justify-between group",
                                isSelected ? getColorClasses(p.color, true) : "bg-card border-border/50 hover:border-border hover:bg-accent/50",
                                !canSelect && "opacity-60 grayscale-[0.5]"
                            )}
                        >
                            <div className="flex gap-4">
                                <div className={cn(
                                    "p-3 rounded-xl transition-colors",
                                    getIconClasses(p.color, isSelected)
                                )}>
                                    <Icon className="w-6 h-6" />
                                </div>
                                <div className="space-y-1">
                                    <div className="flex items-center gap-2">
                                        <h3 className="font-bold text-lg">{p.name}</h3>
                                        {isSelected && <CheckCircle className="w-4 h-4 text-emerald-500" />}
                                    </div>
                                    <p className="text-sm text-muted-foreground leading-relaxed max-w-md">
                                        {p.description}
                                    </p>
                                </div>
                            </div>

                            <div className="flex flex-col items-end gap-2 shrink-0">
                                {p.configured ? (
                                    <span className="flex items-center gap-1.5 px-2.5 py-1 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 rounded-full text-[10px] font-bold uppercase tracking-wider border border-emerald-500/20">
                                        <ShieldCheck className="w-3 h-3" />
                                        Ready
                                    </span>
                                ) : (
                                    <div className="flex flex-col items-end gap-1">
                                        <span className="flex items-center gap-1.5 px-2.5 py-1 bg-amber-500/10 text-amber-600 dark:text-amber-400 rounded-full text-[10px] font-bold uppercase tracking-wider border border-amber-500/20">
                                            <ShieldAlert className="w-3 h-3" />
                                            Action Required
                                        </span>
                                        <span className="text-[9px] font-bold text-muted-foreground group-hover:text-primary transition-colors flex items-center gap-1">
                                            <KeyRound className="w-2.5 h-2.5" /> SET SECRET
                                        </span>
                                    </div>
                                )}
                            </div>
                        </button>
                    );
                })}
            </div>

            <div className="p-6 rounded-2xl border border-primary/10 bg-primary/5 flex gap-4 items-center">
                <div className="p-3 bg-primary/10 rounded-xl">
                    <Info className="w-6 h-6 text-primary" />
                </div>
                <div className="space-y-1">
                    <p className="text-sm font-bold">Scope of Selection</p>
                    <p className="text-xs text-muted-foreground leading-relaxed">
                        This setting controls the inference engine for <span className="text-foreground font-medium">Standard Chat</span> and
                        <span className="text-foreground font-medium"> Rig Agent (Auto Mode)</span>.
                        The OpenClaw Agent (Gateway) maintains its own separate brain selection in the Gateway control panel.
                    </p>
                </div>
            </div>
        </div>
    );
}
