
import { useState, useEffect } from 'react';
import { commands } from '../../lib/bindings';
import { Eye, EyeOff, Save, ShieldCheck, ShieldAlert, Bot, Search, Loader2, Trash2, Plus, Key, X, Radio, KeyRound } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { useConfig } from '../../hooks/use-config';
import { reloadSecrets } from '../../lib/openclaw';

interface SecretCardProps {
    title: string;
    description: string;
    icon: React.ReactNode;
    placeholder: string;
    hasKey: boolean;
    granted: boolean;
    onSave: (key: string) => Promise<void>;
    onToggle: (granted: boolean) => Promise<void>;
    isVisible?: boolean;
    onVisibilityToggle?: (visible: boolean) => Promise<void>;
    onFetch: () => Promise<string | null>;
    onDelete: () => Promise<void>;
    getKeyUrl?: string;
}

function SecretCard({
    title, description, icon, placeholder, hasKey, granted, isVisible, onVisibilityToggle, onSave, onToggle, onFetch, onDelete, getKeyUrl
}: SecretCardProps) {
    const [key, setKey] = useState('');
    const [showKey, setShowKey] = useState(false);
    const [loading, setLoading] = useState(false);
    const [fetching, setFetching] = useState(false);
    const [showConfirm, setShowConfirm] = useState(false);

    // When showing key, if it's empty but we know we have a key, fetch it
    useEffect(() => {
        if (showKey && hasKey && !key) {
            handleFetch();
        }
    }, [showKey, hasKey]);

    const handleFetch = async () => {
        setFetching(true);
        try {
            const res = await onFetch();
            if (res !== null) setKey(res);
        } catch (e) {
            console.error("Failed to fetch key:", e);
        } finally {
            setFetching(false);
        }
    };

    const handleSave = async () => {
        if (!key) return;
        setLoading(true);
        try {
            await onSave(key);
            setKey('');
            setShowKey(false);
            toast.success(`${title} saved`);
            // Trigger hot-reload so IronClaw picks up the new key (fire-and-forget)
            reloadSecrets().catch(() => {/* best-effort */ });
        } catch (e) {
            toast.error(`Failed to save ${title}`);
        } finally {
            setLoading(false);
        }
    };

    const handleDelete = async () => {
        setLoading(true);
        try {
            await onDelete();
            setKey('');
            setShowKey(false);
            setShowConfirm(false);
            toast.success(`${title} deleted`);
        } catch (e) {
            console.error(`[SecretCard] Delete failed for ${title}:`, e);
            toast.error(`Failed to delete ${title}`);
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="p-6 border border-border/50 rounded-2xl bg-card/40 hover:bg-card/60 transition-all duration-300 space-y-4">
            <div className="flex items-start justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2 bg-primary/10 rounded-lg">
                        {icon}
                    </div>
                    <div>
                        <div className="flex items-center gap-2">
                            <h3 className="font-semibold text-lg">{title}</h3>
                            {getKeyUrl && (
                                <button
                                    type="button"
                                    onClick={() => commands.openUrl(getKeyUrl)}
                                    className="text-[10px] font-medium text-primary hover:underline flex items-center gap-0.5 bg-primary/5 px-1.5 py-0.5 rounded border border-primary/10 hover:bg-primary/10 transition-colors cursor-pointer"
                                    title="Open API Key Page"
                                >
                                    Get Key
                                    <Search className="w-2.5 h-2.5" />
                                </button>
                            )}
                        </div>
                        <p className="text-sm text-muted-foreground">{description}</p>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    {hasKey ? (
                        <div className="flex items-center gap-1.5 px-2.5 py-1 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 rounded-full text-xs font-bold uppercase tracking-wider border border-emerald-500/10">
                            <ShieldCheck className="w-3.5 h-3.5" />
                            Configured
                        </div>
                    ) : (
                        <div className="flex items-center gap-1.5 px-2.5 py-1 bg-amber-500/5 text-amber-600 dark:text-amber-400 rounded-full text-xs font-bold uppercase tracking-wider border border-amber-500/10">
                            <ShieldAlert className="w-3.5 h-3.5" />
                            Missing
                        </div>
                    )}

                    {hasKey && !showConfirm && (
                        <button
                            type="button"
                            onClick={() => setShowConfirm(true)}
                            disabled={loading}
                            className="p-1.5 text-muted-foreground hover:text-rose-600 hover:bg-rose-500/10 rounded-lg transition-colors cursor-pointer"
                            title="Delete Key"
                        >
                            <Trash2 className="w-4 h-4" />
                        </button>
                    )}

                    {showConfirm && (
                        <div className="flex items-center gap-2 animate-in fade-in slide-in-from-right-2 duration-200">
                            <button
                                type="button"
                                onClick={() => setShowConfirm(false)}
                                disabled={loading}
                                className="px-2 py-1 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
                            >
                                Cancel
                            </button>
                            <button
                                type="button"
                                onClick={handleDelete}
                                disabled={loading}
                                className="px-2.5 py-1 bg-rose-700 text-white rounded-md text-xs font-bold uppercase tracking-wider hover:bg-rose-800 transition-colors shadow-sm flex items-center gap-1.5 cursor-pointer"
                            >
                                {loading && <Loader2 className="w-3 h-3 animate-spin" />}
                                Confirm Delete
                            </button>
                        </div>
                    )}
                </div>
            </div>

            <div className="flex gap-3 max-w-2xl">
                <div className="relative flex-1">
                    <input
                        type={showKey ? "text" : "password"}
                        value={key}
                        onChange={(e) => setKey(e.target.value)}
                        placeholder={hasKey ? "••••••••••••••••" : placeholder}
                        className="w-full h-11 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm pr-12 font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                    <button
                        onClick={() => setShowKey(!showKey)}
                        disabled={fetching}
                        className="absolute right-3.5 top-3 text-muted-foreground hover:text-foreground transition-colors"
                    >
                        {fetching ? <Loader2 className="w-4 h-4 animate-spin" /> : (showKey ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />)}
                    </button>
                </div>
                <button
                    onClick={handleSave}
                    disabled={loading || !key}
                    className={cn(
                        "px-6 h-11 rounded-xl bg-primary text-primary-foreground font-bold text-xs uppercase tracking-wider flex items-center gap-2 hover:bg-primary/90 transition-all shrink-0 shadow-sm hover:translate-y-[-1px]",
                        (loading || !key) && "opacity-50 cursor-not-allowed transform-none"
                    )}
                >
                    {loading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
                    {hasKey ? "Update" : "Save"}
                </button>
            </div>

            {hasKey && (
                <div className="pt-4 border-t border-border/50 space-y-4">
                    {onVisibilityToggle && (
                        <div className="flex items-center justify-between">
                            <div>
                                <div className="text-sm font-medium">Show models in browser</div>
                                <div className="text-xs text-muted-foreground">Keep the model list clean by hiding inactive providers</div>
                            </div>
                            <button
                                onClick={() => onVisibilityToggle?.(!isVisible)}
                                className={cn(
                                    "relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2",
                                    isVisible ? "bg-emerald-500" : "bg-slate-200 dark:bg-muted"
                                )}
                            >
                                <span
                                    className={cn(
                                        "inline-block h-4 w-4 transform rounded-full bg-white transition-transform ring-0",
                                        isVisible ? "translate-x-6" : "translate-x-1"
                                    )}
                                />
                            </button>
                        </div>
                    )}

                    <div className="flex items-center justify-between">
                        <div>
                            <div className="text-sm font-medium">Access for OpenClaw Agents</div>
                            <div className="text-xs text-muted-foreground">Allow agents to use this key for autonomous tasks</div>
                        </div>
                        <button
                            onClick={() => onToggle(!granted)}
                            className={cn(
                                "relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2",
                                granted ? "bg-primary" : "bg-slate-200 dark:bg-muted"
                            )}
                        >
                            <span
                                className={cn(
                                    "inline-block h-4 w-4 transform rounded-full bg-white transition-transform ring-0",
                                    granted ? "translate-x-6" : "translate-x-1"
                                )}
                            />
                        </button>
                    </div>
                </div>
            )}
        </div>
    );
}

function BedrockCredentialsCard({ status, loadStatus, handleToggle }: {
    status: any;
    loadStatus: () => Promise<void>;
    handleToggle: (secret: string, granted: boolean) => Promise<void>;
}) {
    const [accessKeyId, setAccessKeyId] = useState('');
    const [secretAccessKey, setSecretAccessKey] = useState('');
    const [region, setRegion] = useState('us-east-1');
    const [loading, setLoading] = useState(false);
    const [showKeys, setShowKeys] = useState(false);
    const [fetching, setFetching] = useState(false);
    const [showConfirm, setShowConfirm] = useState(false);

    const hasKey = !!(status?.has_bedrock_key ?? (status as any)?.hasBedrockKey);
    const granted = !!(status?.bedrock_granted ?? (status as any)?.bedrockGranted);

    const handleFetch = async () => {
        setFetching(true);
        try {
            const res = await commands.openclawGetBedrockCredentials();
            if (res.status === 'ok' && res.data) {
                const [ak, sk, r] = res.data;
                if (ak) setAccessKeyId(ak);
                if (sk) setSecretAccessKey(sk);
                if (r) setRegion(r);
            }
            setShowKeys(true);
        } catch {
            toast.error('Failed to fetch Bedrock credentials');
        } finally {
            setFetching(false);
        }
    };

    const handleSave = async () => {
        setLoading(true);
        try {
            const res = await commands.openclawSaveBedrockCredentials(accessKeyId, secretAccessKey, region);
            if (res.status === 'ok') {
                await loadStatus();
                toast.success('Bedrock credentials saved');
            } else {
                toast.error('Failed to save Bedrock credentials');
            }
        } finally {
            setLoading(false);
        }
    };

    const handleDelete = async () => {
        setLoading(true);
        try {
            const res = await commands.openclawSaveBedrockCredentials('', '', '');
            if (res.status === 'ok') {
                setAccessKeyId('');
                setSecretAccessKey('');
                setRegion('us-east-1');
                setShowConfirm(false);
                await loadStatus();
                toast.success('Bedrock credentials deleted');
            } else {
                toast.error('Failed to delete Bedrock credentials');
            }
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="space-y-4 p-5 bg-card/50 rounded-2xl border border-border/50">
            <div className="flex items-start justify-between">
                <div className="flex items-center gap-3">
                    <div className={cn(
                        "w-10 h-10 rounded-xl flex items-center justify-center transition-all",
                        hasKey ? "bg-amber-500/20" : "bg-muted"
                    )}>
                        <Bot className="w-5 h-5 text-amber-500" />
                    </div>
                    <div>
                        <div className="font-semibold text-foreground flex items-center gap-2">
                            Amazon Bedrock
                            {hasKey && (
                                <span className={cn(
                                    "text-[10px] px-2 py-0.5 rounded-full font-bold uppercase tracking-wider",
                                    granted ? "bg-emerald-500/20 text-emerald-500" : "bg-amber-500/20 text-amber-500"
                                )}>
                                    {granted ? 'Active' : 'Paused'}
                                </span>
                            )}
                        </div>
                        <div className="text-xs text-muted-foreground mt-0.5">
                            Access Claude, Llama, Nova and other models via AWS Bedrock.
                        </div>
                    </div>
                </div>

                <div className="flex items-center gap-1.5">
                    {!showKeys && hasKey && (
                        <button onClick={handleFetch} disabled={fetching} className="p-1.5 text-muted-foreground hover:text-foreground rounded-lg transition-colors cursor-pointer" title="Show Credentials">
                            {fetching ? <Loader2 className="w-4 h-4 animate-spin" /> : <Eye className="w-4 h-4" />}
                        </button>
                    )}
                    {showKeys && (
                        <button onClick={() => { setShowKeys(false); setAccessKeyId(''); setSecretAccessKey(''); }} className="p-1.5 text-muted-foreground hover:text-foreground rounded-lg transition-colors cursor-pointer" title="Hide">
                            <EyeOff className="w-4 h-4" />
                        </button>
                    )}
                    {hasKey && !showConfirm && (
                        <button onClick={() => setShowConfirm(true)} disabled={loading} className="p-1.5 text-muted-foreground hover:text-rose-600 hover:bg-rose-500/10 rounded-lg transition-colors cursor-pointer" title="Delete">
                            <Trash2 className="w-4 h-4" />
                        </button>
                    )}
                    {showConfirm && (
                        <div className="flex items-center gap-2 animate-in fade-in slide-in-from-right-2 duration-200">
                            <button onClick={() => setShowConfirm(false)} disabled={loading} className="px-2 py-1 text-xs font-medium text-muted-foreground hover:text-foreground transition-colors cursor-pointer">Cancel</button>
                            <button onClick={handleDelete} disabled={loading} className="px-2.5 py-1 bg-rose-700 text-white rounded-md text-xs font-bold uppercase tracking-wider hover:bg-rose-800 transition-colors shadow-sm flex items-center gap-1.5 cursor-pointer">
                                {loading && <Loader2 className="w-3 h-3 animate-spin" />}
                                Confirm Delete
                            </button>
                        </div>
                    )}
                </div>
            </div>

            <div className="space-y-3 max-w-2xl">
                <div>
                    <label className="text-xs font-medium text-muted-foreground mb-1 block">AWS Access Key ID</label>
                    <input
                        type={showKeys ? "text" : "password"}
                        value={accessKeyId}
                        onChange={(e) => setAccessKeyId(e.target.value)}
                        placeholder={hasKey ? "••••••••••••••••" : "AKIA..."}
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>
                <div>
                    <label className="text-xs font-medium text-muted-foreground mb-1 block">AWS Secret Access Key</label>
                    <input
                        type="password"
                        value={secretAccessKey}
                        onChange={(e) => setSecretAccessKey(e.target.value)}
                        placeholder={hasKey ? "••••••••••••••••" : "wJal..."}
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>
                <div>
                    <label className="text-xs font-medium text-muted-foreground mb-1 block">AWS Region</label>
                    <input
                        type="text"
                        value={region}
                        onChange={(e) => setRegion(e.target.value)}
                        placeholder="us-east-1"
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>
            </div>

            <div className="flex items-center gap-3">
                <button
                    onClick={handleSave}
                    disabled={loading || (!accessKeyId && !secretAccessKey)}
                    className={cn(
                        "px-6 h-10 rounded-xl bg-primary text-primary-foreground font-bold text-xs uppercase tracking-wider flex items-center gap-2 hover:bg-primary/90 transition-all shrink-0 shadow-sm hover:translate-y-[-1px]",
                        (loading || (!accessKeyId && !secretAccessKey)) && "opacity-50 cursor-not-allowed transform-none"
                    )}
                >
                    {loading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
                    {hasKey ? "Update" : "Save"}
                </button>
                <a
                    href="https://console.aws.amazon.com/iam/home#/security_credentials"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-xs text-primary hover:underline"
                >
                    Get AWS Credentials →
                </a>
            </div>

            {hasKey && (
                <div className="pt-3 border-t border-border/50">
                    <div className="flex items-center justify-between">
                        <div>
                            <div className="text-sm font-medium">Access for OpenClaw Agents</div>
                            <div className="text-xs text-muted-foreground">Allow OpenClaw to use Bedrock for inference</div>
                        </div>
                        <button
                            onClick={() => handleToggle('amazon-bedrock', !granted)}
                            className={cn(
                                "relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-2",
                                granted ? "bg-emerald-500" : "bg-slate-200 dark:bg-muted"
                            )}
                        >
                            <span
                                className={cn(
                                    "inline-block h-4 w-4 transform rounded-full bg-white transition-transform ring-0",
                                    granted ? "translate-x-6" : "translate-x-1"
                                )}
                            />
                        </button>
                    </div>
                </div>
            )}
        </div>
    );
}

function AddSecretForm({ onAdd }: { onAdd: (name: string, value: string, description: string | null) => Promise<void> }) {
    const [name, setName] = useState('');
    const [value, setValue] = useState('');
    const [description, setDescription] = useState('');
    const [loading, setLoading] = useState(false);
    const [isOpen, setIsOpen] = useState(false);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!name.trim() || !value.trim()) return;
        setLoading(true);
        try {
            await onAdd(name.trim(), value.trim(), description.trim() || null);
            setName('');
            setValue('');
            setDescription('');
            setIsOpen(false);
        } catch (e) {
            // Error handled by parent toast
        } finally {
            setLoading(false);
        }
    };

    if (!isOpen) {
        return (
            <button
                onClick={() => setIsOpen(true)}
                className="w-full p-6 border border-dashed border-border/60 rounded-2xl flex items-center justify-center gap-2 text-muted-foreground hover:text-foreground hover:border-primary/50 hover:bg-primary/5 transition-all group"
            >
                <Plus className="w-5 h-5 group-hover:scale-110 transition-transform" />
                <span className="font-bold uppercase tracking-wider text-xs">Add Custom API Secret</span>
            </button>
        );
    }

    return (
        <form onSubmit={handleSubmit} className="p-6 border border-border/50 rounded-2xl bg-card/60 backdrop-blur-md shadow-2xl space-y-6 animate-in fade-in zoom-in-95 duration-200">
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 font-semibold">
                    <Key className="w-4 h-4 text-primary" />
                    Add New Secret
                </div>
                <button type="button" onClick={() => setIsOpen(false)} className="p-1 hover:bg-muted rounded-md">
                    <X className="w-4 h-4 text-muted-foreground" />
                </button>
            </div>

            <div className="grid gap-5">
                <div className="space-y-2">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 ml-1">Secret Name</label>
                    <input
                        autoFocus
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="e.g. OpenAI, ElevenLabs, etc."
                        className="w-full h-11 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                        required
                    />
                </div>
                <div className="space-y-2">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 ml-1">Description (Optional)</label>
                    <input
                        value={description}
                        onChange={(e) => setDescription(e.target.value)}
                        placeholder="What is this key used for?"
                        className="w-full h-11 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>
                <div className="space-y-2">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 ml-1">Secret Token / Value</label>
                    <input
                        type="password"
                        value={value}
                        onChange={(e) => setValue(e.target.value)}
                        placeholder="Paste your key here"
                        className="w-full h-11 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                        required
                    />
                </div>
            </div>

            <div className="flex justify-end gap-3 pt-2">
                <button
                    type="button"
                    onClick={() => setIsOpen(false)}
                    className="px-6 h-10 rounded-xl text-xs font-bold uppercase tracking-wider hover:bg-muted transition-colors"
                >
                    Cancel
                </button>
                <button
                    disabled={loading || !name || !value}
                    className="px-6 h-10 rounded-xl bg-primary text-primary-foreground font-bold text-xs uppercase tracking-wider flex items-center gap-2 hover:bg-primary/90 transition-all shadow-sm hover:translate-y-[-1px] disabled:opacity-50 disabled:transform-none"
                >
                    {loading && <Loader2 className="w-4 h-4 animate-spin" />}
                    Save Secret
                </button>
            </div>
        </form>
    );
}

export function SecretsTab() {
    const [status, setStatus] = useState<any>(null);
    const { config, updateConfig } = useConfig();
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        loadData();
    }, []);

    const loadData = async () => {
        try {
            const sRes = await commands.openclawGetStatus();
            if (sRes.status === 'ok') setStatus(sRes.data);
        } catch (e) {
            console.error(e);
        } finally {
            setLoading(false);
        }
    };

    const loadStatus = loadData;

    const toggleProviderVisibility = async (provider: string, visible: boolean) => {
        if (!config) return;
        let disabled = [...(config.disabled_providers || [])];
        if (visible) {
            disabled = disabled.filter(p => p !== provider);
        } else {
            if (!disabled.includes(provider)) disabled.push(provider);
        }
        const newConfig = { ...config, disabled_providers: disabled };
        await updateConfig(newConfig);
        toast.success(`${provider.charAt(0).toUpperCase() + provider.slice(1)} models ${visible ? 'enabled' : 'disabled'}`);
    };

    const isProviderVisible = (provider: string) => {
        return !config?.disabled_providers?.includes(provider);
    };

    const handleAnthropicSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.openclawSaveAnthropicKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('anthropic', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleBraveSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.openclawSaveBraveKey(value);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenAISave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.openclawSaveOpenaiKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('openai', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenRouterSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.openclawSaveOpenrouterKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('openrouter', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGeminiSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.openclawSaveGeminiKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('gemini', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGroqSave = async (key: string) => {
        const value = key.trim() || null;
        const res = await commands.openclawSaveGroqKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('groq', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleToggle = async (secret: string, granted: boolean) => {
        try {
            const res = await commands.openclawToggleSecretAccess(secret, granted);
            if (res.status === 'ok') {
                await loadStatus();
                toast.success(`Access ${granted ? 'granted' : 'revoked'}`);
            } else {
                toast.error("Failed to update access: " + res.error);
            }
        } catch (e) {
            toast.error("Failed to update access");
        }
    };

    const handleAnthropicFetch = async (): Promise<string | null> => {
        const res = await commands.openclawGetAnthropicKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleBraveFetch = async (): Promise<string | null> => {
        const res = await commands.openclawGetBraveKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleOpenAIFetch = async (): Promise<string | null> => {
        const res = await commands.openclawGetOpenaiKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleOpenRouterFetch = async (): Promise<string | null> => {
        const res = await commands.openclawGetOpenrouterKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleGeminiFetch = async (): Promise<string | null> => {
        const res = await commands.openclawGetGeminiKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleGroqFetch = async (): Promise<string | null> => {
        const res = await commands.openclawGetGroqKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleAnthropicDelete = async () => {
        const res = await commands.openclawSaveAnthropicKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleBraveDelete = async () => {
        const res = await commands.openclawSaveBraveKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenAIDelete = async () => {
        const res = await commands.openclawSaveOpenaiKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenRouterDelete = async () => {
        const res = await commands.openclawSaveOpenrouterKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGeminiDelete = async () => {
        const res = await commands.openclawSaveGeminiKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGroqDelete = async () => {
        const res = await commands.openclawSaveGroqKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleAddCustomSecret = async (name: string, value: string, description: string | null) => {
        const res = await commands.openclawAddCustomSecret(name, value, description);
        if (res.status === 'ok') {
            await loadStatus();
            toast.success(`${name} secret added`);
        } else {
            toast.error("Failed to add secret: " + res.error);
            throw new Error(res.error);
        }
    };

    const handleRemoveCustomSecret = async (id: string) => {
        const res = await commands.openclawRemoveCustomSecret(id);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            toast.error("Failed to remove secret: " + res.error);
        }
    };

    const handleToggleCustomSecret = async (id: string, granted: boolean) => {
        const res = await commands.openclawToggleCustomSecret(id, granted);
        if (res.status === 'ok') {
            await loadStatus();
            toast.success(`Access ${granted ? 'granted' : 'revoked'}`);
        } else {
            toast.error("Failed to update access: " + res.error);
        }
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center p-20">
                <Loader2 className="w-8 h-8 animate-spin text-primary/50" />
            </div>
        );
    }

    return (
        <div className="space-y-6 pb-20">
            <div className="flex flex-col gap-1">
                <h2 className="text-2xl font-bold">API Secrets & Security</h2>
                <p className="text-muted-foreground">Manage your credentials and control agent access permissions.</p>
            </div>

            <div className="grid gap-8">
                {/* Inference Providers Section */}
                <div className="space-y-6">
                    <div className="flex items-center justify-between border-b border-border/50 pb-4">
                        <div className="flex items-center gap-2">
                            <Bot className="w-5 h-5 text-primary" />
                            <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-foreground">Inference Cloud Brains</h3>
                        </div>
                        <button
                            onClick={() => window.dispatchEvent(new CustomEvent('open-settings', { detail: 'inference' }))}
                            className="text-[10px] font-bold text-primary hover:text-primary/80 transition-colors flex items-center gap-2 bg-primary/5 px-3 py-1.5 rounded-lg border border-primary/10 group"
                        >
                            <Radio className="w-3.5 h-3.5 group-hover:scale-110 transition-transform" />
                            SET CLOUD CHAT BRAIN
                        </button>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="Anthropic API Key"
                            description="Used for Claude 4.5 Sonnet / Opus and other world-class models."
                            icon={<Bot className="w-5 h-5 text-purple-500" />}
                            placeholder="sk-ant-api03-..."
                            hasKey={!!(status?.has_anthropic_key ?? status?.hasAnthropicKey)}
                            granted={!!(status?.anthropic_granted ?? status?.anthropicGranted)}
                            isVisible={isProviderVisible('anthropic')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('anthropic', v)}
                            onSave={handleAnthropicSave}
                            onToggle={(g) => handleToggle('anthropic', g)}
                            onFetch={handleAnthropicFetch}
                            onDelete={handleAnthropicDelete}
                            getKeyUrl="https://console.anthropic.com/settings/keys"
                        />

                        <SecretCard
                            title="OpenAI API Key"
                            description="For GPT 5.2, specialized reasoning and advanced coding models."
                            icon={<Bot className="w-5 h-5 text-emerald-500" />}
                            placeholder="sk-..."
                            hasKey={!!(status?.has_openai_key ?? status?.hasOpenaiKey)}
                            granted={!!(status?.openai_granted ?? status?.openaiGranted)}
                            isVisible={isProviderVisible('openai')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('openai', v)}
                            onSave={handleOpenAISave}
                            onToggle={(g) => handleToggle('openai', g)}
                            onFetch={handleOpenAIFetch}
                            onDelete={handleOpenAIDelete}
                            getKeyUrl="https://platform.openai.com/api-keys"
                        />

                        <SecretCard
                            title="OpenRouter API Key"
                            description="Universal access to hundreds of open-source and proprietary models."
                            icon={<Bot className="w-5 h-5 text-indigo-500" />}
                            placeholder="sk-or-v1-..."
                            hasKey={!!(status?.has_openrouter_key ?? status?.hasOpenrouterKey)}
                            granted={!!(status?.openrouter_granted ?? status?.openrouterGranted)}
                            isVisible={isProviderVisible('openrouter')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('openrouter', v)}
                            onSave={handleOpenRouterSave}
                            onToggle={(g) => handleToggle('openrouter', g)}
                            onFetch={handleOpenRouterFetch}
                            onDelete={handleOpenRouterDelete}
                            getKeyUrl="https://openrouter.ai/keys"
                        />

                        <SecretCard
                            title="Google Gemini API Key"
                            description="Native access to Gemini 2.0 Flash, Pro and Google's latest frontier models."
                            icon={<Bot className="w-5 h-5 text-cyan-500" />}
                            placeholder="AIza..."
                            hasKey={!!(status?.has_gemini_key ?? status?.hasGeminiKey)}
                            granted={!!(status?.gemini_granted ?? status?.geminiGranted)}
                            isVisible={isProviderVisible('gemini')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('gemini', v)}
                            onSave={handleGeminiSave}
                            onToggle={(g) => handleToggle('gemini', g)}
                            onFetch={handleGeminiFetch}
                            onDelete={handleGeminiDelete}
                            getKeyUrl="https://aistudio.google.com/app/apikey"
                        />

                        <SecretCard
                            title="Groq API Key"
                            description="Ultra-fast inference for Llama 3, Mixtral and other open weights models."
                            icon={<Bot className="w-5 h-5 text-orange-400" />}
                            placeholder="gsk_..."
                            hasKey={!!(status?.has_groq_key ?? status?.hasGroqKey)}
                            granted={!!(status?.groq_granted ?? status?.groqGranted)}
                            isVisible={isProviderVisible('groq')}
                            onVisibilityToggle={(v) => toggleProviderVisibility('groq', v)}
                            onSave={handleGroqSave}
                            onToggle={(g) => handleToggle('groq', g)}
                            onFetch={handleGroqFetch}
                            onDelete={handleGroqDelete}
                            getKeyUrl="https://console.groq.com/keys"
                        />
                    </div>
                </div>

                {/* Additional Cloud Providers Section */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/50 pb-4">
                        <Bot className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">Additional Cloud Providers</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="xAI API Key"
                            description="Access Grok models for reasoning and code generation."
                            icon={<Bot className="w-5 h-5 text-blue-400" />}
                            placeholder="xai-..."
                            hasKey={!!(status?.has_xai_key ?? (status as any)?.hasXaiKey)}
                            granted={!!(status?.xai_granted ?? (status as any)?.xaiGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('xai', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('xAI key saved'); }
                                else toast.error('Failed to save xAI key');
                            }}
                            onToggle={(g) => handleToggle('xai', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('xai');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('xai', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete xAI key');
                            }}
                            getKeyUrl="https://console.x.ai/"
                        />

                        <SecretCard
                            title="Mistral AI API Key"
                            description="Access Mistral Large, Medium, and other Mistral models."
                            icon={<Bot className="w-5 h-5 text-amber-500" />}
                            placeholder="..."
                            hasKey={!!(status?.has_mistral_key ?? (status as any)?.hasMistralKey)}
                            granted={!!(status?.mistral_granted ?? (status as any)?.mistralGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('mistral', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Mistral key saved'); }
                                else toast.error('Failed to save Mistral key');
                            }}
                            onToggle={(g) => handleToggle('mistral', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('mistral');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('mistral', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Mistral key');
                            }}
                            getKeyUrl="https://console.mistral.ai/api-keys/"
                        />

                        <SecretCard
                            title="Venice AI API Key"
                            description="Privacy-focused AI inference with uncensored models."
                            icon={<Bot className="w-5 h-5 text-teal-500" />}
                            placeholder="..."
                            hasKey={!!(status?.has_venice_key ?? (status as any)?.hasVeniceKey)}
                            granted={!!(status?.venice_granted ?? (status as any)?.veniceGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('venice', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Venice key saved'); }
                                else toast.error('Failed to save Venice key');
                            }}
                            onToggle={(g) => handleToggle('venice', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('venice');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('venice', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Venice key');
                            }}
                            getKeyUrl="https://venice.ai/settings/api"
                        />

                        <SecretCard
                            title="Together AI API Key"
                            description="Access open-source models with fast serverless inference."
                            icon={<Bot className="w-5 h-5 text-violet-500" />}
                            placeholder="..."
                            hasKey={!!(status?.has_together_key ?? (status as any)?.hasTogetherKey)}
                            granted={!!(status?.together_granted ?? (status as any)?.togetherGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('together', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Together key saved'); }
                                else toast.error('Failed to save Together key');
                            }}
                            onToggle={(g) => handleToggle('together', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('together');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('together', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Together key');
                            }}
                            getKeyUrl="https://api.together.xyz/settings/api-keys"
                        />

                        <SecretCard
                            title="Moonshot API Key"
                            description="Kimi-powered long-context models with strong multilingual support."
                            icon={<Bot className="w-5 h-5 text-slate-400" />}
                            placeholder="..."
                            hasKey={!!(status?.has_moonshot_key ?? (status as any)?.hasMoonshotKey)}
                            granted={!!(status?.moonshot_granted ?? (status as any)?.moonshotGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('moonshot', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Moonshot key saved'); }
                                else toast.error('Failed to save Moonshot key');
                            }}
                            onToggle={(g) => handleToggle('moonshot', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('moonshot');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('moonshot', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Moonshot key');
                            }}
                            getKeyUrl="https://platform.moonshot.cn/"
                        />

                        <SecretCard
                            title="MiniMax API Key"
                            description="Access MiniMax models for text and multimodal generation."
                            icon={<Bot className="w-5 h-5 text-rose-400" />}
                            placeholder="..."
                            hasKey={!!(status?.has_minimax_key ?? (status as any)?.hasMinimaxKey)}
                            granted={!!(status?.minimax_granted ?? (status as any)?.minimaxGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('minimax', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('MiniMax key saved'); }
                                else toast.error('Failed to save MiniMax key');
                            }}
                            onToggle={(g) => handleToggle('minimax', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('minimax');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('minimax', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete MiniMax key');
                            }}
                        />

                        <SecretCard
                            title="NVIDIA NIM API Key"
                            description="Enterprise-grade inference for NVIDIA-optimized models."
                            icon={<Bot className="w-5 h-5 text-green-500" />}
                            placeholder="nvapi-..."
                            hasKey={!!(status?.has_nvidia_key ?? (status as any)?.hasNvidiaKey)}
                            granted={!!(status?.nvidia_granted ?? (status as any)?.nvidiaGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('nvidia', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('NVIDIA key saved'); }
                                else toast.error('Failed to save NVIDIA key');
                            }}
                            onToggle={(g) => handleToggle('nvidia', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('nvidia');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('nvidia', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete NVIDIA key');
                            }}
                            getKeyUrl="https://build.nvidia.com/"
                        />

                        <SecretCard
                            title="Baidu Qianfan API Key"
                            description="Access ERNIE and other Baidu AI models."
                            icon={<Bot className="w-5 h-5 text-sky-500" />}
                            placeholder="..."
                            hasKey={!!(status?.has_qianfan_key ?? (status as any)?.hasQianfanKey)}
                            granted={!!(status?.qianfan_granted ?? (status as any)?.qianfanGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('qianfan', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Qianfan key saved'); }
                                else toast.error('Failed to save Qianfan key');
                            }}
                            onToggle={(g) => handleToggle('qianfan', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('qianfan');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('qianfan', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Qianfan key');
                            }}
                        />

                        <SecretCard
                            title="Xiaomi MiLM API Key"
                            description="Access Xiaomi's MiLM language models."
                            icon={<Bot className="w-5 h-5 text-orange-500" />}
                            placeholder="..."
                            hasKey={!!(status?.has_xiaomi_key ?? (status as any)?.hasXiaomiKey)}
                            granted={!!(status?.xiaomi_granted ?? (status as any)?.xiaomiGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('xiaomi', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Xiaomi key saved'); }
                                else toast.error('Failed to save Xiaomi key');
                            }}
                            onToggle={(g) => handleToggle('xiaomi', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('xiaomi');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('xiaomi', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Xiaomi key');
                            }}
                        />

                        <SecretCard
                            title="Cohere API Key"
                            description="Access Command R+ for chat and embed-multilingual for RAG embeddings."
                            icon={<Bot className="w-5 h-5 text-fuchsia-500" />}
                            placeholder="..."
                            hasKey={!!(status?.has_cohere_key ?? (status as any)?.hasCohereKey)}
                            granted={!!(status?.cohere_granted ?? (status as any)?.cohereGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('cohere', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Cohere key saved'); }
                                else toast.error('Failed to save Cohere key');
                            }}
                            onToggle={(g) => handleToggle('cohere', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('cohere');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('cohere', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Cohere key');
                            }}
                            getKeyUrl="https://dashboard.cohere.com/api-keys"
                        />

                        <SecretCard
                            title="Voyage AI API Key"
                            description="High-quality embedding models for advanced RAG and semantic search."
                            icon={<Bot className="w-5 h-5 text-sky-400" />}
                            placeholder="pa-..."
                            hasKey={!!(status?.has_voyage_key ?? (status as any)?.hasVoyageKey)}
                            granted={!!(status?.voyage_granted ?? (status as any)?.voyageGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('voyage', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Voyage key saved'); }
                                else toast.error('Failed to save Voyage key');
                            }}
                            onToggle={(g) => handleToggle('voyage', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('voyage');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('voyage', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Voyage key');
                            }}
                            getKeyUrl="https://dash.voyageai.com/api-keys"
                        />
                    </div>
                </div>

                {/* Speech & Image Generation Section */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/10 pb-4">
                        <Radio className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">Speech & Image Generation</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="Deepgram API Key"
                            description="Cloud speech-to-text — fast and accurate transcription with Nova-2."
                            icon={<Bot className="w-5 h-5 text-green-400" />}
                            placeholder="dg_..."
                            hasKey={!!((status as any)?.has_deepgram_key ?? (status as any)?.hasDeepgramKey)}
                            granted={!!((status as any)?.deepgram_granted ?? (status as any)?.deepgramGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('deepgram', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Deepgram key saved'); }
                                else toast.error('Failed to save Deepgram key');
                            }}
                            onToggle={(g) => handleToggle('deepgram', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('deepgram');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('deepgram', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Deepgram key');
                            }}
                            getKeyUrl="https://console.deepgram.com/"
                        />

                        <SecretCard
                            title="ElevenLabs API Key"
                            description="Cloud text-to-speech — natural voices with emotional range."
                            icon={<Bot className="w-5 h-5 text-violet-400" />}
                            placeholder="sk_..."
                            hasKey={!!((status as any)?.has_elevenlabs_key ?? (status as any)?.hasElevenlabsKey)}
                            granted={!!((status as any)?.elevenlabs_granted ?? (status as any)?.elevenlabsGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('elevenlabs', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('ElevenLabs key saved'); }
                                else toast.error('Failed to save ElevenLabs key');
                            }}
                            onToggle={(g) => handleToggle('elevenlabs', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('elevenlabs');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('elevenlabs', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete ElevenLabs key');
                            }}
                            getKeyUrl="https://elevenlabs.io/app/settings/api-keys"
                        />

                        <SecretCard
                            title="Stability AI API Key"
                            description="Cloud image generation — SDXL Turbo, Stable Diffusion 3, and more."
                            icon={<Bot className="w-5 h-5 text-rose-400" />}
                            placeholder="sk-..."
                            hasKey={!!((status as any)?.has_stability_key ?? (status as any)?.hasStabilityKey)}
                            granted={!!((status as any)?.stability_granted ?? (status as any)?.stabilityGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('stability', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('Stability AI key saved'); }
                                else toast.error('Failed to save Stability AI key');
                            }}
                            onToggle={(g) => handleToggle('stability', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('stability');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('stability', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete Stability AI key');
                            }}
                            getKeyUrl="https://platform.stability.ai/account/keys"
                        />

                        <SecretCard
                            title="fal.ai API Key"
                            description="Cloud image generation — FLUX, SDXL, fast inference via serverless GPU."
                            icon={<Bot className="w-5 h-5 text-amber-400" />}
                            placeholder="fal_..."
                            hasKey={!!((status as any)?.has_fal_key ?? (status as any)?.hasFalKey)}
                            granted={!!((status as any)?.fal_granted ?? (status as any)?.falGranted)}
                            onSave={async (key) => {
                                const res = await commands.openclawSaveImplicitProviderKey('fal', key);
                                if (res.status === 'ok') { await loadStatus(); toast.success('fal.ai key saved'); }
                                else toast.error('Failed to save fal.ai key');
                            }}
                            onToggle={(g) => handleToggle('fal', g)}
                            onFetch={async () => {
                                const res = await commands.openclawGetImplicitProviderKey('fal');
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSaveImplicitProviderKey('fal', '');
                                if (res.status === 'ok') await loadStatus();
                                else toast.error('Failed to delete fal.ai key');
                            }}
                            getKeyUrl="https://fal.ai/dashboard/keys"
                        />
                    </div>
                </div>

                {/* Amazon Bedrock Section (uses AWS credentials, not a single API key) */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/50 pb-4">
                        <Bot className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">Amazon Bedrock (AWS)</h3>
                    </div>

                    <BedrockCredentialsCard
                        status={status}
                        loadStatus={loadStatus}
                        handleToggle={handleToggle}
                    />
                </div>

                {/* System & Data Tools Section */}
                <div className="space-y-6">
                    <div className="flex items-center gap-2 border-b border-border/10 pb-4">
                        <KeyRound className="w-5 h-5 text-muted-foreground" />
                        <h3 className="text-sm font-bold uppercase tracking-[0.1em] text-muted-foreground">System & Data Tools</h3>
                    </div>

                    <div className="grid gap-6">
                        <SecretCard
                            title="Brave Search API Key"
                            description="Enables web search, current news, and weather tools for agents."
                            icon={<Search className="w-5 h-5 text-orange-500" />}
                            placeholder="BSA..."
                            hasKey={!!(status?.has_brave_key ?? status?.hasBraveKey)}
                            granted={!!(status?.brave_granted ?? status?.braveGranted)}
                            onSave={handleBraveSave}
                            onToggle={(g) => handleToggle('brave', g)}
                            onFetch={handleBraveFetch}
                            onDelete={handleBraveDelete}
                            getKeyUrl="https://brave.com/search/api/"
                        />

                        <SecretCard
                            title="Hugging Face Token"
                            description="Required for downloading gated models and datasets."
                            icon={<Bot className="w-5 h-5 text-yellow-500" />}
                            placeholder="hf_..."
                            hasKey={!!(status?.has_huggingface_token ?? status?.hasHuggingfaceToken)}
                            granted={!!(status?.huggingface_granted ?? status?.huggingfaceGranted)}
                            onSave={async (key) => {
                                const value = key.trim() || "";
                                const res = await commands.openclawSetHfToken(value);
                                if (res.status === 'ok') {
                                    await loadStatus();
                                    toast.success("Hugging Face token saved");
                                } else {
                                    console.error("Failed to save HF token:", res);
                                    toast.error("Failed to save HF token");
                                }
                            }}
                            onToggle={(g) => handleToggle('huggingface', g)}
                            onFetch={async () => {
                                const res = await commands.getHfToken();
                                return res.status === 'ok' ? res.data : null;
                            }}
                            onDelete={async () => {
                                const res = await commands.openclawSetHfToken("");
                                if (res.status === 'ok') await loadStatus();
                                else toast.error("Failed to delete HF token");
                            }}
                            getKeyUrl="https://huggingface.co/settings/tokens"
                        />
                    </div>
                </div>

                {status?.custom_secrets && status.custom_secrets.length > 0 && (
                    <div className="space-y-6 pt-4">
                        <div className="flex items-center gap-2">
                            <Key className="w-4 h-4 text-muted-foreground" />
                            <h3 className="text-sm font-medium uppercase tracking-wider text-muted-foreground">Custom Secrets</h3>
                        </div>
                        <div className="grid gap-6">
                            {status.custom_secrets.map((secret: any) => (
                                <SecretCard
                                    key={secret.id}
                                    title={secret.name}
                                    description={secret.description || "Custom API Secret"}
                                    icon={<Key className="w-5 h-5 text-blue-500" />}
                                    placeholder="••••••••••••••••"
                                    hasKey={true}
                                    granted={secret.granted}
                                    onSave={async (_key) => {
                                        // Update logic for custom secrets? 
                                        // For now let's just use it as is, maybe disable update if we don't have it implemented.
                                        toast.info("Update for custom secrets not implemented yet, please re-add if needed.");
                                    }}
                                    onToggle={(g) => handleToggleCustomSecret(secret.id, g)}
                                    onFetch={async () => secret.value} // Value is already in the status for custom ones
                                    onDelete={() => handleRemoveCustomSecret(secret.id)}
                                />
                            ))}
                        </div>
                    </div>
                )}

                <div className="pt-4 border-t border-border/50">
                    <AddSecretForm onAdd={handleAddCustomSecret} />
                </div>
            </div>

            <div className="p-4 rounded-xl border border-primary/10 bg-primary/5 text-muted-foreground text-sm flex gap-3 items-center">
                <ShieldCheck className="w-5 h-5 shrink-0 text-emerald-600 dark:text-emerald-400" />
                <p>
                    <span className="font-bold text-foreground">Privacy First:</span> Your secrets are stored in a secure local directory and
                    <strong> strictly isolated</strong> from the agent process unless access is explicitly granted above.
                </p>
            </div>
        </div>
    );
}
