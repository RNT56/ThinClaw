import { useState } from 'react';
import { commands, type OpenClawStatus } from '../../lib/bindings';
import { Eye, EyeOff, Save, Bot, Loader2, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';

export function BedrockCredentialsCard({ status, loadStatus, handleToggle }: {
    status: OpenClawStatus | null;
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

    const hasKey = !!status?.has_bedrock_key;
    const granted = !!status?.bedrock_granted;

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
                            <div className="text-sm font-medium">Access for ThinClaw Agents</div>
                            <div className="text-xs text-muted-foreground">Allow ThinClaw to use Bedrock for inference</div>
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
