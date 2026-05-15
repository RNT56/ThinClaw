import { useState, useEffect } from 'react';
import { commands } from '../../lib/bindings';
import { Eye, EyeOff, Save, ShieldCheck, ShieldAlert, Loader2, Trash2, Search } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { reloadSecrets } from '../../lib/thinclaw';

export interface SecretCardProps {
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

export function SecretCard({
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
            // Trigger hot-reload so ThinClaw picks up the new key (fire-and-forget)
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
                            <div className="text-sm font-medium">Access for ThinClaw Agents</div>
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
