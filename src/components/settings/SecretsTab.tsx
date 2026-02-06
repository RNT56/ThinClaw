
import { useState, useEffect } from 'react';
import { commands } from '../../lib/bindings';
import { Eye, EyeOff, Save, ShieldCheck, ShieldAlert, Bot, Search, Loader2, Trash2, Plus, Key, X, Radio, KeyRound } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';

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
}

function SecretCard({
    title, description, icon, placeholder, hasKey, granted, isVisible, onVisibilityToggle, onSave, onToggle, onFetch, onDelete
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
        } catch (e) {
            toast.error(`Failed to save ${title}`);
        } finally {
            setLoading(false);
        }
    };

    const handleDelete = async () => {
        console.log(`[SecretCard] Deleting ${title}...`);
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
                        <h3 className="font-semibold text-lg">{title}</h3>
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
    const [config, setConfig] = useState<any>(null);
    const [loading, setLoading] = useState(true);

    useEffect(() => {
        loadData();
    }, []);

    const loadData = async () => {
        try {
            const [sRes, cRes] = await Promise.all([
                commands.getClawdbotStatus(),
                commands.getUserConfig()
            ]);
            if (sRes.status === 'ok') setStatus(sRes.data);
            setConfig(cRes);
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
        await commands.updateUserConfig(newConfig);
        setConfig(newConfig);
        toast.success(`${provider.charAt(0).toUpperCase() + provider.slice(1)} models ${visible ? 'enabled' : 'disabled'}`);
    };

    const isProviderVisible = (provider: string) => {
        return !config?.disabled_providers?.includes(provider);
    };

    const handleAnthropicSave = async (key: string) => {
        const value = key.trim() || null;
        console.log(`[SecretsTab] Saving Anthropic key:`, value ? "REDACTED" : "null");
        const res = await commands.saveAnthropicKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('anthropic', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleBraveSave = async (key: string) => {
        const value = key.trim() || null;
        console.log(`[SecretsTab] Saving Brave key:`, value ? "REDACTED" : "null");
        const res = await commands.saveBraveKey(value);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenAISave = async (key: string) => {
        const value = key.trim() || null;
        console.log(`[SecretsTab] Saving OpenAI key:`, value ? "REDACTED" : "null");
        const res = await commands.saveOpenaiKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('openai', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenRouterSave = async (key: string) => {
        const value = key.trim() || null;
        console.log(`[SecretsTab] Saving OpenRouter key:`, value ? "REDACTED" : "null");
        const res = await commands.saveOpenrouterKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('openrouter', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGeminiSave = async (key: string) => {
        const value = key.trim() || null;
        console.log(`[SecretsTab] Saving Gemini key:`, value ? "REDACTED" : "null");
        const res = await commands.saveGeminiKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('gemini', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGroqSave = async (key: string) => {
        const value = key.trim() || null;
        console.log(`[SecretsTab] Saving Groq key:`, value ? "REDACTED" : "null");
        const res = await commands.saveGroqKey(value);
        if (res.status === 'ok') {
            if (value) await toggleProviderVisibility('groq', true);
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleToggle = async (secret: string, granted: boolean) => {
        try {
            const res = await commands.clawdbotToggleSecretAccess(secret, granted);
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
        const res = await commands.getAnthropicKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleBraveFetch = async (): Promise<string | null> => {
        const res = await commands.getBraveKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleOpenAIFetch = async (): Promise<string | null> => {
        const res = await commands.getOpenaiKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleOpenRouterFetch = async (): Promise<string | null> => {
        const res = await commands.getOpenrouterKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleGeminiFetch = async (): Promise<string | null> => {
        const res = await commands.getGeminiKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleGroqFetch = async (): Promise<string | null> => {
        const res = await commands.getGroqKey();
        return res.status === 'ok' ? res.data : null;
    };

    const handleAnthropicDelete = async () => {
        console.log(`[SecretsTab] Deleting Anthropic key...`);
        const res = await commands.saveAnthropicKey(null);
        console.log(`[SecretsTab] Anthropic save result:`, res);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleBraveDelete = async () => {
        console.log(`[SecretsTab] Deleting Brave key...`);
        const res = await commands.saveBraveKey(null);
        console.log(`[SecretsTab] Brave save result:`, res);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenAIDelete = async () => {
        console.log(`[SecretsTab] Deleting OpenAI key...`);
        const res = await commands.saveOpenaiKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleOpenRouterDelete = async () => {
        console.log(`[SecretsTab] Deleting OpenRouter key...`);
        const res = await commands.saveOpenrouterKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGeminiDelete = async () => {
        console.log(`[SecretsTab] Deleting Gemini key...`);
        const res = await commands.saveGeminiKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleGroqDelete = async () => {
        console.log(`[SecretsTab] Deleting Groq key...`);
        const res = await commands.saveGroqKey(null);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            throw new Error(res.error);
        }
    };

    const handleAddCustomSecret = async (name: string, value: string, description: string | null) => {
        const res = await commands.addCustomSecret(name, value, description);
        if (res.status === 'ok') {
            await loadStatus();
            toast.success(`${name} secret added`);
        } else {
            toast.error("Failed to add secret: " + res.error);
            throw new Error(res.error);
        }
    };

    const handleRemoveCustomSecret = async (id: string) => {
        const res = await commands.removeCustomSecret(id);
        if (res.status === 'ok') {
            await loadStatus();
        } else {
            toast.error("Failed to remove secret: " + res.error);
        }
    };

    const handleToggleCustomSecret = async (id: string, granted: boolean) => {
        const res = await commands.clawdbotToggleCustomSecret(id, granted);
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
                        />
                    </div>
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
                                console.log(`[SecretsTab] Saving HF token:`, value ? "REDACTED" : "empty (delete)");
                                const res = await commands.setHfToken(value);
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
                                const res = await commands.setHfToken("");
                                if (res.status === 'ok') await loadStatus();
                                else toast.error("Failed to delete HF token");
                            }}
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
