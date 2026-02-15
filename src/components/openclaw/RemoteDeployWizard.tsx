import React, { useState, useRef, useEffect } from 'react';
import { commands } from '../../lib/bindings';
import { listen } from '@tauri-apps/api/event';
import { Server, CheckCircle, AlertCircle, Loader2, Zap } from 'lucide-react';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

interface RemoteDeployWizardProps {
    isOpen: boolean;
    onCheckStatus?: () => void;
    onClose: () => void;
}

export const RemoteDeployWizard: React.FC<RemoteDeployWizardProps> = ({ isOpen, onCheckStatus, onClose }) => {
    const [step, setStep] = useState<'form' | 'deploying' | 'success' | 'error'>('form');
    const [ip, setIp] = useState('');
    const [user, setUser] = useState('root');
    const [logs, setLogs] = useState<string[]>([]);
    const [errorMsg, setErrorMsg] = useState('');
    const [deployMode, setDeployMode] = useState<'new' | 'existing'>('new');
    const [existingUrl, setExistingUrl] = useState('');
    const [existingToken, setExistingToken] = useState('');

    const logEndRef = useRef<HTMLDivElement>(null);

    // Auto-scroll logs
    useEffect(() => {
        logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }, [logs]);

    const startDeploy = async () => {
        if (!ip) return;

        setStep('deploying');
        setLogs(['Starting deployment script...', `Target: ${user}@${ip}`]);
        setErrorMsg('');

        try {
            // Listen for log events
            const unlistenLog = await listen<string>('deploy-log', (event) => {
                setLogs((prev) => [...prev, event.payload]);
            });

            const unlistenStatus = await listen<string>('deploy-status', (event) => {
                if (event.payload === 'success') {
                    setStep('success');
                } else {
                    setErrorMsg(event.payload);
                    setStep('error');
                }
                // Cleanup listeners
                unlistenLog();
                unlistenStatus();
            });

            // Invoke backend command
            await commands.openclawDeployRemote(ip, user);

        } catch (e: any) {
            setErrorMsg(typeof e === 'string' ? e : e.message);
            setStep('error');
        }
    };

    const handleConnect = async () => {
        // Attempt to extract tailscale IP from logs if possible, or just use the target IP provided
        // Ideally the script outputs "Connect to: ws://..." and we parse it

        // For now, assume public IP or user manually corrects it
        const targetUrl = `ws://${ip}:18789`;

        try {
            // Create a new profile for this deployed agent
            const newProfile: openclaw.AgentProfile = {
                id: crypto.randomUUID(),
                name: `Remote Agent (${ip})`,
                url: targetUrl,
                token: null, // Token is likely empty or needs manual entry if script generated one
                mode: 'remote',
                auto_connect: true
            };

            await openclaw.addAgentProfile(newProfile);
            await openclaw.saveGatewaySettings('remote', targetUrl, '');
        } catch (e) {
            console.error("Failed to add profile:", e);
            // Fallback
            await commands.openclawSaveGatewaySettings('remote', targetUrl, '');
        }

        onCheckStatus?.();
        onClose();
    };

    const handleDirectConnect = async () => {
        if (!existingUrl) return;

        // Normalize URL
        let url = existingUrl.trim();
        if (!url.startsWith('ws://') && !url.startsWith('wss://')) {
            url = `ws://${url}`; // Default to ws
        }
        // If just IP/Host, append default port if missing
        if (!url.split('://')[1].includes(':')) {
            url = `${url}:18789`;
        }

        try {
            const newProfile: openclaw.AgentProfile = {
                id: crypto.randomUUID(),
                name: `Agent (${url.replace('ws://', '').replace('wss://', '').split(':')[0]})`,
                url: url,
                token: existingToken || null,
                mode: 'remote',
                auto_connect: true
            };

            await openclaw.addAgentProfile(newProfile);
            await openclaw.saveGatewaySettings('remote', url, existingToken || '');

            onCheckStatus?.();
            onClose();
            toast.success('Connected to remote agent');
        } catch (e) {
            console.error("Failed to connect:", e);
            toast.error("Failed to connect to agent");
        }
    };

    if (!isOpen) return null;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-200">
            <div className="bg-background/95 border border-border rounded-2xl shadow-2xl w-full max-w-2xl flex flex-col max-h-[90vh] overflow-hidden">
                {/* Header */}
                <div className="p-6 border-b border-border flex items-center gap-4 bg-muted/30">
                    <div className="w-12 h-12 rounded-xl bg-primary/10 flex items-center justify-center text-primary shadow-sm">
                        <Server className="w-6 h-6" />
                    </div>
                    <div>
                        <h2 className="text-xl font-bold tracking-tight text-foreground">Remote Agent Manager</h2>
                        <p className="text-sm font-medium text-muted-foreground">Deploy new agents or connect to existing clusters</p>
                    </div>
                </div>

                {/* Content */}
                <div className="flex-1 overflow-y-auto p-6 scrollbar-thin scrollbar-thumb-border scrollbar-track-transparent">

                    {step === 'form' && (
                        <div className="space-y-6">
                            {/* Tabs */}
                            <div className="flex bg-muted p-1.5 rounded-xl">
                                <button
                                    onClick={() => setDeployMode('new')}
                                    className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${deployMode === 'new' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
                                >
                                    Deploy New Agent
                                </button>
                                <button
                                    onClick={() => setDeployMode('existing')}
                                    className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${deployMode === 'existing' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
                                >
                                    Connect Existing
                                </button>
                            </div>

                            {deployMode === 'new' ? (
                                <div className="space-y-6 animate-in fade-in slide-in-from-left-4 duration-300">
                                    <div className="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 text-sm text-blue-600 dark:text-blue-400">
                                        <h4 className="font-bold mb-1 flex items-center gap-2"><AlertCircle size={16} /> Prerequisites</h4>
                                        <ul className="list-disc list-inside space-y-1 opacity-90 text-xs font-medium">
                                            <li>A fresh Ubuntu/Debian Linux server.</li>
                                            <li>SSH Access (Keys recommended).</li>
                                            <li>Local <code>ansible</code> (Script will try to auto-install on macOS).</li>
                                        </ul>
                                    </div>

                                    <div className="grid gap-4">
                                        <div className="space-y-2">
                                            <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Server IP Address</label>
                                            <input
                                                type="text"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                                placeholder="e.g. 192.168.1.50"
                                                value={ip}
                                                onChange={(e) => setIp(e.target.value)}
                                            />
                                        </div>

                                        <div className="space-y-2">
                                            <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">SSH User</label>
                                            <input
                                                type="text"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                                placeholder="root"
                                                value={user}
                                                onChange={(e) => setUser(e.target.value)}
                                            />
                                            <p className="text-[10px] text-muted-foreground font-medium">Usually <code>root</code> or <code>ubuntu</code>.</p>
                                        </div>
                                    </div>
                                </div>
                            ) : (
                                <div className="space-y-6 animate-in fade-in slide-in-from-right-4 duration-300">
                                    <div className="bg-emerald-500/10 border border-emerald-500/20 rounded-xl p-4 text-sm text-emerald-600 dark:text-emerald-400">
                                        <h4 className="font-bold mb-1 flex items-center gap-2"><CheckCircle size={16} /> Direct Connection</h4>
                                        <p className="opacity-90 text-xs font-medium">Connect to an already running OpenClaw instance via WebSocket.</p>
                                    </div>

                                    <div className="space-y-2">
                                        <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Agent URL / IP</label>
                                        <div className="relative">
                                            <input
                                                type="text"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-emerald-500/20 outline-none transition-all font-mono pl-10 placeholder:text-muted-foreground/50"
                                                placeholder="192.168.1.50 or ws://..."
                                                value={existingUrl}
                                                onChange={(e) => setExistingUrl(e.target.value)}
                                            />
                                            <Server className="absolute left-3 top-3.5 w-4 h-4 text-muted-foreground" />
                                        </div>
                                        <p className="text-[10px] text-muted-foreground font-medium">We'll automatically add <code>:18789</code> if omitted.</p>
                                    </div>

                                    <div className="space-y-2">
                                        <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Auth Token (Optional)</label>
                                        <input
                                            type="password"
                                            className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-emerald-500/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                            placeholder="••••••••"
                                            value={existingToken}
                                            onChange={(e) => setExistingToken(e.target.value)}
                                        />
                                    </div>
                                </div>
                            )}
                        </div>
                    )}

                    {(step === 'deploying' || step === 'error' || step === 'success') && (
                        <div className="space-y-4 h-full flex flex-col">
                            <div className="flex items-center justify-between text-sm border-b border-border pb-2">
                                <span className="text-muted-foreground font-bold uppercase tracking-wider text-xs">Deployment Log</span>
                                {step === 'deploying' && (
                                    <div className="flex items-center gap-2 text-blue-500 font-bold text-xs">
                                        <Loader2 className="animate-spin w-3 h-3" /> In Progress
                                    </div>
                                )}
                                {step === 'success' && <span className="text-emerald-500 flex items-center gap-1 font-bold text-xs"><CheckCircle className="w-3 h-3" /> Complete</span>}
                                {step === 'error' && <span className="text-rose-500 flex items-center gap-1 font-bold text-xs"><AlertCircle className="w-3 h-3" /> Failed</span>}
                            </div>

                            <div className="flex-1 bg-black/90 rounded-xl border border-border/50 p-4 font-mono text-[10px] leading-relaxed overflow-y-auto min-h-[300px] max-h-[400px] shadow-inner">
                                {logs.map((log, i) => (
                                    <div key={i} className={`mb-0.5 whitespace-pre-wrap ${log.includes('[stderr]') ? 'text-amber-400' : 'text-zinc-400'}`}>
                                        <span className="opacity-50 select-none mr-2">[{new Date().toLocaleTimeString()}]</span>
                                        {log.replace(/^\[std...\] /, '')}
                                    </div>
                                ))}
                                {step === 'error' && (
                                    <div className="mt-2 text-rose-500 font-bold border-t border-rose-500/20 pt-2">Error: {errorMsg}</div>
                                )}
                                <div ref={logEndRef} />
                            </div>
                        </div>
                    )}

                </div>

                {/* Footer */}
                <div className="p-6 border-t border-border bg-muted/30 flex justify-end gap-3">
                    {step === 'form' && (
                        <>
                            <button
                                onClick={onClose}
                                className="px-5 py-2.5 rounded-xl text-muted-foreground hover:text-foreground hover:bg-muted transition-colors text-sm font-bold"
                            >
                                Cancel
                            </button>
                            <button
                                onClick={deployMode === 'new' ? startDeploy : handleDirectConnect}
                                disabled={deployMode === 'new' ? !ip : !existingUrl}
                                className={`px-6 py-2.5 rounded-xl text-white text-sm font-bold shadow-lg transition-all flex items-center gap-2 ${deployMode === 'new'
                                    ? 'bg-blue-600 hover:bg-blue-500 disabled:bg-blue-600/50 shadow-blue-500/20'
                                    : 'bg-emerald-600 hover:bg-emerald-500 disabled:bg-emerald-600/50 shadow-emerald-500/20'
                                    } disabled:opacity-50 disabled:cursor-not-allowed`}
                            >
                                {deployMode === 'new' ? <Server className="w-4 h-4" /> : <Zap className="w-4 h-4" />}
                                {deployMode === 'new' ? 'Start Deployment' : 'Connect Agent'}
                            </button>
                        </>
                    )}

                    {step === 'deploying' && (
                        <button
                            disabled
                            className="px-6 py-2.5 rounded-xl bg-muted text-muted-foreground text-sm font-bold cursor-wait flex items-center gap-2"
                        >
                            <Loader2 className="w-4 h-4 animate-spin" />
                            Deploying...
                        </button>
                    )}

                    {step === 'success' && (
                        <>
                            <button
                                onClick={onClose}
                                className="px-5 py-2.5 rounded-xl text-muted-foreground hover:text-foreground hover:bg-muted transition-colors text-sm font-bold"
                            >
                                Close
                            </button>
                            <button
                                onClick={handleConnect}
                                className="px-6 py-2.5 rounded-xl bg-emerald-600 hover:bg-emerald-500 text-white text-sm font-bold shadow-lg shadow-emerald-500/20 transition-all flex items-center gap-2"
                            >
                                <CheckCircle className="w-4 h-4" />
                                Connect & Save
                            </button>
                        </>
                    )}

                    {step === 'error' && (
                        <button
                            onClick={() => setStep('form')}
                            className="px-6 py-2.5 rounded-xl bg-muted hover:bg-muted/80 text-foreground text-sm font-bold transition-all"
                        >
                            Try Again
                        </button>
                    )}
                </div>

            </div>
        </div>
    );
};
