import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import { Radio, Play, Square, RefreshCw, Shield, AlertTriangle, CheckCircle, XCircle, Copy, Zap, Code, Monitor, Server, RotateCcw, Trash2, MousePointerClick } from 'lucide-react';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import * as clawdbot from '../../lib/clawdbot';
import { useModelContext } from '../model-context';
// Removed unused import

interface GatewayTabProps {
    className?: string;
}

type GatewayStatus = 'stopped' | 'starting' | 'running' | 'error';

interface PermissionStatus {
    accessibility: boolean;
    screen_recording: boolean;
}

interface StatusInfo {
    gateway: GatewayStatus;
    wsConnected: boolean;
    slackEnabled: boolean;
    telegramEnabled: boolean;
    port: number;
    gatewayMode: string;
    remoteUrl: string | null;
    remoteToken: string | null;
    deviceId: string;
    authToken: string;
    stateDir: string;
    nodeHostEnabled: boolean;
    localInferenceEnabled: boolean;
    exposeInference: boolean;
}

export function GatewayTab({ className }: GatewayTabProps) {
    const [status, setStatus] = useState<StatusInfo>({
        gateway: 'stopped',
        wsConnected: false,
        slackEnabled: false,
        telegramEnabled: false,
        port: 18789,
        gatewayMode: 'local',
        remoteUrl: null,
        remoteToken: null,
        deviceId: '',
        authToken: '',
        stateDir: '',
        nodeHostEnabled: false,
        localInferenceEnabled: false,
        exposeInference: false
    });

    const [permissions, setPermissions] = useState<PermissionStatus>({
        accessibility: false,
        screen_recording: false
    });

    const [remoteUrlInput, setRemoteUrlInput] = useState('');
    const [remoteTokenInput, setRemoteTokenInput] = useState('');
    const [viewingFile, setViewingFile] = useState<{ title: string; content: string } | null>(null);

    const { maxContext, setMaxContext } = useModelContext();
    const [showContextWarning, setShowContextWarning] = useState(false);

    const [isLoading, setIsLoading] = useState(false);

    // Calculate Mode
    const isSafeMode = status.gatewayMode === 'local';

    // Poll gateway status
    const fetchStatus = useCallback(async () => {
        try {
            const s = await clawdbot.getClawdbotStatus();
            setStatus({
                gateway: s.gateway_running ? 'running' : 'stopped',
                wsConnected: s.ws_connected,
                slackEnabled: s.slack_enabled,
                telegramEnabled: s.telegram_enabled,
                port: s.port,
                gatewayMode: s.gateway_mode,
                remoteUrl: s.remote_url,
                remoteToken: s.remote_token,
                deviceId: s.device_id,
                authToken: s.auth_token,
                stateDir: s.state_dir,
                nodeHostEnabled: s.node_host_enabled,
                localInferenceEnabled: s.local_inference_enabled,
                exposeInference: (s as any).expose_inference || false // Handle potentially missing field during update
            });

            const perms = await clawdbot.getPermissionStatus();
            setPermissions(perms);

            if (s.remote_url) setRemoteUrlInput(s.remote_url);
            if (s.remote_token) setRemoteTokenInput(s.remote_token);
        } catch (e) {
            console.error('Failed to fetch clawdbot status:', e);
        }
    }, []);

    useEffect(() => {
        fetchStatus();
        const interval = setInterval(fetchStatus, 3000);
        return () => clearInterval(interval);
    }, [fetchStatus]);

    const executeStartGateway = async () => {
        setIsLoading(true);
        setStatus(s => ({ ...s, gateway: 'starting' }));

        try {
            await clawdbot.startClawdbotGateway();
            await fetchStatus();
            toast.success('Clawdbot Gateway started');
        } catch (e) {
            console.error('Failed to start gateway:', e);
            setStatus(s => ({ ...s, gateway: 'error' }));
            toast.error('Failed to start gateway', { description: String(e) });
        } finally {
            setIsLoading(false);
        }
    };

    const handleStart = async () => {
        if (status.gatewayMode === 'local' && maxContext < 32768) {
            setShowContextWarning(true);
            return;
        }
        await executeStartGateway();
    };

    const handleStop = async () => {
        setIsLoading(true);

        try {
            await clawdbot.stopClawdbotGateway();
            await fetchStatus();
            toast.info('Clawdbot Gateway stopped');
        } catch (e) {
            console.error('Failed to stop gateway:', e);
            toast.error('Failed to stop gateway', { description: String(e) });
        } finally {
            setIsLoading(false);
        }
    };

    const handleRestart = async () => {
        await handleStop();
        await new Promise(r => setTimeout(r, 500));
        await handleStart();
    };

    const handleSaveGateway = async (mode: string, url: string | null, token: string | null) => {
        try {
            await clawdbot.saveGatewaySettings(mode, url, token);
            await fetchStatus();
            toast.success('Gateway settings updated');
        } catch (e) {
            toast.error('Failed to update gateway settings', { description: String(e) });
        }
    };

    const copyToClipboard = (text: string, label: string) => {
        navigator.clipboard.writeText(text);
        toast.success(`${label} copied to clipboard`);
    };

    const copyDiagnostics = async () => {
        try {
            const diag = await clawdbot.getClawdbotDiagnostics();
            navigator.clipboard.writeText(JSON.stringify(diag, null, 2));
            toast.success('Diagnostics copied to clipboard');
        } catch (e) {
            // Fallback to local diagnostics
            const fallback = {
                timestamp: new Date().toISOString(),
                gateway: status.gateway,
                wsConnected: status.wsConnected,
                port: status.port,
                platform: navigator.platform,
                version: '0.1.0'
            };
            navigator.clipboard.writeText(JSON.stringify(fallback, null, 2));
            toast.success('Diagnostics copied (local fallback)');
        }
    };

    const StatusDashboard = () => (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="p-6 rounded-2xl bg-gradient-to-br from-card to-background border border-border/50 shadow-xl relative overflow-hidden group">
                <div className="absolute top-0 right-0 p-3 opacity-10 group-hover:opacity-20 transition-opacity">
                    <Radio className="w-12 h-12 text-primary" />
                </div>
                <div className="relative z-10 flex flex-col justify-between h-full space-y-4">
                    <div className="space-y-1">
                        <span className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Service Orbit</span>
                        <h4 className="text-xl font-bold tracking-tight">Gateway Engine</h4>
                    </div>
                    <div className="flex items-center justify-between">
                        <div className="flex flex-col">
                            <span className="text-xs text-muted-foreground font-medium">WebSocket Plane</span>
                            <span className="text-lg font-mono font-bold">127.0.0.1:{status.port}</span>
                        </div>
                        <StatusBadge state={status.gateway} />
                    </div>
                </div>
            </div>

            <div className="p-6 rounded-2xl bg-gradient-to-br from-card to-background border border-border/50 shadow-xl relative overflow-hidden group">
                <div className="absolute top-0 right-0 p-3 opacity-10 group-hover:opacity-20 transition-opacity">
                    <CheckCircle className="w-12 h-12 text-emerald-500" />
                </div>
                <div className="relative z-10 flex flex-col justify-between h-full space-y-4">
                    <div className="space-y-1">
                        <span className="text-[10px] font-bold text-emerald-600 dark:text-emerald-400 uppercase tracking-[0.2em]">Connection Pulse</span>
                        <h4 className="text-xl font-bold tracking-tight">Moltbot Sync</h4>
                    </div>
                    <div className="flex items-center justify-between">
                        <div className="flex flex-col">
                            <span className="text-xs text-muted-foreground font-medium">Control Channel</span>
                            <span className="text-lg font-mono font-bold">{status.wsConnected ? "ACTIVE" : "OFFLINE"}</span>
                        </div>
                        <div className={cn(
                            "flex items-center gap-2 px-4 py-1.5 rounded-full text-xs font-bold transition-all border",
                            status.wsConnected
                                ? "bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 border-emerald-500/10"
                                : "bg-muted text-muted-foreground border-border shadow-none"
                        )}>
                            <div className={cn("w-1.5 h-1.5 rounded-full", status.wsConnected ? "bg-emerald-500" : "bg-muted-foreground")} />
                            {status.wsConnected ? 'SYNCED' : 'DISCONNECTED'}
                        </div>
                    </div>
                </div>
            </div>
        </div>
    );

    const ModeCard = () => (
        <div className="flex p-1.5 rounded-2xl bg-muted/30 border border-border/50 shadow-inner group transition-all hover:bg-muted/50">
            <button
                onClick={() => handleSaveGateway('local', null, null)}
                className={cn(
                    "flex-1 flex flex-col items-center py-4 rounded-xl text-sm font-bold transition-all gap-1",
                    status.gatewayMode === 'local'
                        ? "bg-card shadow-xl text-primary border border-primary/20 scale-[1.02]"
                        : "text-muted-foreground hover:text-foreground opacity-60 hover:opacity-100"
                )}
            >
                <Monitor className="w-5 h-5" />
                <span>Local Sidecar</span>
            </button>
            <button
                onClick={() => handleSaveGateway('remote', remoteUrlInput, remoteTokenInput)}
                className={cn(
                    "flex-1 flex flex-col items-center py-4 rounded-xl text-sm font-bold transition-all gap-1",
                    status.gatewayMode === 'remote'
                        ? "bg-card shadow-xl text-primary border border-primary/20 scale-[1.02]"
                        : "text-muted-foreground hover:text-foreground opacity-60 hover:opacity-100"
                )}
            >
                <Server className="w-5 h-5" />
                <span>Remote Bridge</span>
            </button>
        </div>
    );

    const ControlPanel = () => (
        <div className="flex gap-3">
            {status.gateway === 'stopped' || status.gateway === 'error' ? (
                <button
                    onClick={handleStart}
                    disabled={isLoading}
                    className={cn(
                        "flex-1 py-4 rounded-2xl font-bold transition-all shadow-lg hover:translate-y-[-1px] active:translate-y-[1px]",
                        "flex items-center justify-center gap-3",
                        "bg-emerald-600 dark:bg-emerald-700 text-white shadow-emerald-900/10",
                        isLoading && "opacity-50 cursor-wait"
                    )}
                >
                    <Play className="w-5 h-5 fill-current" />
                    Engage Gateway
                </button>
            ) : (
                <>
                    <button
                        onClick={handleStop}
                        disabled={isLoading}
                        className={cn(
                            "flex-1 py-4 rounded-2xl font-bold transition-all shadow-lg hover:translate-y-[-1px] active:translate-y-[1px]",
                            "flex items-center justify-center gap-3",
                            "bg-rose-600 dark:bg-rose-700 text-white shadow-rose-900/10",
                            isLoading && "opacity-50 cursor-wait"
                        )}
                    >
                        <Square className="w-5 h-5 fill-current" />
                        Kill Process
                    </button>
                    <button
                        onClick={handleRestart}
                        disabled={isLoading}
                        className={cn(
                            "px-6 py-4 rounded-2xl font-bold transition-all shadow-xl hover:scale-[1.02] active:scale-[0.98]",
                            "flex items-center justify-center gap-2 bg-card border border-border group",
                            isLoading && "opacity-50 cursor-wait"
                        )}
                        title="Pulse Restart"
                    >
                        <RefreshCw className={cn("w-5 h-5 text-primary transition-transform group-hover:rotate-180 duration-500", isLoading && "animate-spin")} />
                    </button>
                </>
            )}
        </div>
    );

    return (
        <motion.div
            initial={{ opacity: 0, scale: 0.98 }}
            animate={{ opacity: 1, scale: 1 }}
            className={cn("space-y-10 pb-20 max-w-4xl mx-auto", className)}
        >
            {/* Unified Status Dashboard */}
            <div className="space-y-6">
                <div className="flex items-center justify-between">
                    <div className="flex items-center gap-4">
                        <div className="p-3 bg-primary/10 rounded-2xl border border-primary/20">
                            <Zap className="w-6 h-6 text-primary" />
                        </div>
                        <div>
                            <h2 className="text-2xl font-bold tracking-tight">Gateway Control</h2>
                            <p className="text-sm text-muted-foreground font-medium">Runtime Management & Connection Orchestration</p>
                        </div>
                    </div>
                </div>

                <StatusDashboard />
            </div>

            {/* Orchestration Controls */}
            <div className="space-y-6 bg-card/30 p-8 rounded-3xl border border-border/50 shadow-sm">
                <div className="flex items-center gap-2 mb-2">
                    <Code className="w-4 h-4 text-primary" />
                    <h3 className="text-sm font-bold uppercase tracking-[0.2em] text-muted-foreground/80">Operational Parameters</h3>
                </div>
                <ModeCard />
                <ControlPanel />
            </div>

            {/* Detailed Configuration */}
            <div className="space-y-8">
                {status.gatewayMode === 'remote' && (
                    <div className="p-8 rounded-3xl bg-card border border-border/50 shadow-xl space-y-6 animate-in fade-in slide-in-from-top-4 duration-500">
                        <div className="flex items-center gap-3 border-b border-border/50 pb-4">
                            <Server className="w-6 h-6 text-indigo-500" />
                            <div>
                                <h4 className="font-bold text-lg">Remote Bridge Connection</h4>
                                <p className="text-xs text-muted-foreground">Linking your desktop client to an external Moltbot instance.</p>
                            </div>
                        </div>
                        <div className="grid grid-cols-1 gap-6">
                            <div className="space-y-2">
                                <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Gateway Socket URL</label>
                                <input
                                    type="text"
                                    placeholder="ws://server-ip:18789"
                                    value={remoteUrlInput}
                                    onChange={(e) => setRemoteUrlInput(e.target.value)}
                                    onBlur={() => handleSaveGateway('remote', remoteUrlInput, remoteTokenInput)}
                                    className="w-full bg-muted/30 border border-border/50 rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none font-mono"
                                />
                            </div>
                            <div className="space-y-2">
                                <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Secure Access Token</label>
                                <input
                                    type="password"
                                    placeholder="••••••••••••••••"
                                    value={remoteTokenInput}
                                    onChange={(e) => setRemoteTokenInput(e.target.value)}
                                    onBlur={() => handleSaveGateway('remote', remoteUrlInput, remoteTokenInput)}
                                    className="w-full bg-muted/30 border border-border/50 rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none"
                                />
                            </div>
                        </div>
                    </div>
                )}

                <div className="p-8 rounded-3xl bg-card border border-border/50 shadow-xl space-y-6">
                    <div className="flex items-center gap-3 border-b border-border/50 pb-4">
                        <Monitor className="w-6 h-6 text-amber-500" />
                        <div>
                            <h4 className="font-bold text-lg">Agent Tuning</h4>
                            <p className="text-xs text-muted-foreground">Configuration for autonomous behavior and intelligence.</p>
                        </div>
                    </div>

                    <div className="bg-amber-500/5 border border-amber-500/20 p-4 rounded-xl flex gap-4">
                        <AlertTriangle className="w-8 h-8 text-amber-500 mt-1 shrink-0" />
                        <p className="text-sm text-amber-700 dark:text-amber-400 leading-relaxed">
                            <span className="font-bold">Intelligence Requirement:</span> Agents function optimally with a <span className="underline">32k minimum context window</span>. Verify your memory settings if you experience session fragmentation.
                        </p>
                    </div>

                    <div className="space-y-2">
                        <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Cloud Brain: Anthropic API Key</label>
                        <input
                            type="password"
                            placeholder="sk-ant-..."
                            className="w-full bg-muted/30 border border-border/50 rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none font-mono"
                            onBlur={(e) => {
                                if (!e.target.value) return;
                                clawdbot.saveAnthropicKey(e.target.value)
                                    .then(() => toast.success('Agent logic key synchronized'))
                                    .catch((err) => toast.error('Sync failed', { description: String(err) }));
                            }}
                        />
                        <p className="text-[10px] text-muted-foreground italic pl-1">Key is ephemeralized and stored securely in identity.json</p>
                    </div>
                </div>

                <div className="p-8 rounded-3xl bg-card border border-border/50 shadow-xl space-y-8">
                    <div className="flex items-center justify-between border-b border-border/50 pb-4">
                        <div className="flex items-center gap-3">
                            <Code className="w-6 h-6 text-blue-500" />
                            <div>
                                <h4 className="font-bold text-lg">Cognitive Manifests</h4>
                                <p className="text-xs text-muted-foreground">Read and refine the core personality files.</p>
                            </div>
                        </div>
                        <button
                            onClick={async () => {
                                if (status.stateDir) {
                                    try {
                                        const baseDir = status.stateDir.replace(/\/state$/, '');
                                        await clawdbot.revealPath(`${baseDir}/workspace`);
                                    } catch (e) { toast.error('Directory access denied'); }
                                }
                            }}
                            className="px-5 py-2 rounded-xl bg-primary/10 hover:bg-primary/20 text-xs font-bold transition-all text-primary border border-primary/20"
                        >
                            Reveal Workspace
                        </button>
                    </div>

                    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                        {[
                            { id: 'IDENTITY.md', label: 'Identity', icon: Shield },
                            { id: 'SOUL.md', label: 'Soul', icon: Zap },
                            { id: 'MEMORY.md', label: 'Chronicles', icon: RotateCcw, memory: true },
                            { id: 'USER.md', label: 'Observer', icon: Monitor }
                        ].map(file => (
                            <button
                                key={file.id}
                                onClick={async () => {
                                    try {
                                        const content = file.memory
                                            ? await clawdbot.getClawdbotMemory()
                                            : await clawdbot.getClawdbotFile(file.id);
                                        setViewingFile({ title: file.id, content });
                                    } catch (e) { toast.error(`Failed to read ${file.id}`); }
                                }}
                                className="flex flex-col items-center gap-3 p-4 rounded-2xl bg-muted/30 hover:bg-muted/50 border border-border/50 transition-all group"
                            >
                                <file.icon className="w-5 h-5 text-muted-foreground group-hover:text-primary transition-colors" />
                                <span className="text-xs font-bold uppercase tracking-wider">{file.label}</span>
                            </button>
                        ))}
                    </div>

                    {viewingFile && (
                        <div className="animate-in fade-in slide-in-from-top-4 duration-500 space-y-3">
                            <div className="flex items-center justify-between px-2">
                                <span className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">{viewingFile.title}</span>
                                <button onClick={() => setViewingFile(null)} className="text-xs font-bold text-muted-foreground hover:text-foreground">Close Editor</button>
                            </div>
                            <textarea
                                readOnly
                                value={viewingFile.content}
                                className="w-full h-80 bg-black/20 dark:bg-black/40 border border-border/50 rounded-2xl p-5 text-xs font-mono text-foreground/80 resize-none shadow-inner outline-none scrollbar-hide"
                            />
                        </div>
                    )}
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                    {/* OS Automation Card */}
                    <div className="p-6 rounded-3xl bg-card border border-border/50 shadow-xl space-y-4 group">
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-3">
                                <div className="p-2 bg-indigo-500/10 rounded-xl group-hover:bg-indigo-500/20 transition-colors">
                                    <Monitor className="w-5 h-5 text-indigo-500" />
                                </div>
                                <h4 className="font-bold text-base">OS Synthesis</h4>
                            </div>
                            <button
                                onClick={async () => {
                                    try {
                                        await clawdbot.toggleClawdbotNodeHost(!status.nodeHostEnabled);
                                        await fetchStatus();
                                        toast.success('automation state toggled');
                                    } catch (e) { toast.error('toggle failed'); }
                                }}
                                className={cn(
                                    "relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-all duration-300",
                                    status.nodeHostEnabled ? "bg-indigo-600" : "bg-muted"
                                )}
                            >
                                <span className={cn(
                                    "inline-block h-5 w-5 rounded-full bg-white shadow-lg transition-transform duration-300",
                                    status.nodeHostEnabled ? "translate-x-5" : "translate-x-0"
                                )} />
                            </button>
                        </div>
                        <p className="text-xs text-muted-foreground leading-relaxed">
                            Deep OS hooks: terminal bridging, filesystem manipulation, and browser puppet control.
                        </p>
                    </div>

                    {/* Local Inference Card */}
                    <div className="p-6 rounded-3xl bg-card border border-border/50 shadow-xl space-y-4 group">
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-3">
                                <div className="p-2 bg-emerald-500/10 rounded-xl group-hover:bg-emerald-500/20 transition-colors">
                                    <Zap className="w-5 h-5 text-emerald-500" />
                                </div>
                                <h4 className="font-bold text-base">Local Neural Link</h4>
                            </div>
                            <button
                                onClick={async () => {
                                    try {
                                        await clawdbot.toggleClawdbotLocalInference(!status.localInferenceEnabled);
                                        await fetchStatus();
                                        toast.success('Inference link state toggled');
                                    } catch (e) { toast.error('Link toggle failed'); }
                                }}
                                className={cn(
                                    "relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-all duration-300",
                                    status.localInferenceEnabled ? "bg-emerald-600" : "bg-muted"
                                )}
                            >
                                <span className={cn(
                                    "inline-block h-5 w-5 rounded-full bg-white shadow-lg transition-transform duration-300",
                                    status.localInferenceEnabled ? "translate-x-5" : "translate-x-0"
                                )} />
                            </button>
                        </div>
                        <p className="text-xs text-muted-foreground leading-relaxed">
                            Expose your high-performance local LLMs to the gateway for zero-latency agentic thought.
                        </p>
                    </div>
                </div>

                {/* Extended Setup & Safety */}
                <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                    {/* Security Manifest */}
                    <div className="p-8 rounded-3xl bg-emerald-500/[0.03] border border-emerald-500/20 space-y-4">
                        <div className="flex items-center gap-3">
                            <Shield className="w-6 h-6 text-emerald-600 dark:text-emerald-400" />
                            <h4 className="font-bold text-lg text-emerald-700 dark:text-emerald-300">Security Vault</h4>
                        </div>
                        <div className="space-y-4">
                            <div className="flex flex-col gap-1.5">
                                <span className="text-[10px] font-bold uppercase tracking-widest text-emerald-500/60">Desktop Handshake Token</span>
                                <div className="flex items-center justify-between bg-black/5 dark:bg-black/20 p-3 rounded-xl border border-border/50">
                                    <span className="text-[11px] font-mono truncate max-w-[180px]">••••••••••••••••••••••••••••••••</span>
                                    <button
                                        onClick={() => copyToClipboard(status.authToken, 'Access Token')}
                                        className="p-1.5 hover:bg-emerald-600/10 rounded-lg transition-colors"
                                    >
                                        <Copy className="w-4 h-4 text-emerald-600 dark:text-emerald-400" />
                                    </button>
                                </div>
                            </div>
                            <p className="text-[11px] text-emerald-600/70 leading-relaxed font-medium">
                                Pure loopback binding (127.0.0.1) enforced. Token authentication required for all IPC/WS requests. Discovery disabled.
                            </p>
                        </div>
                    </div>

                    {/* Critical Factory Reset */}
                    <div className="p-8 rounded-3xl bg-red-500/[0.03] border border-red-500/20 space-y-4 relative overflow-hidden group">
                        <div className="absolute top-0 right-0 p-4 opacity-5 group-hover:opacity-10 transition-opacity">
                            <AlertTriangle className="w-16 h-16 text-red-500" />
                        </div>
                        <div className="relative z-10 space-y-4">
                            <div className="flex items-center gap-3">
                                <Trash2 className="w-6 h-6 text-rose-600" />
                                <h4 className="font-bold text-lg text-rose-700 dark:text-rose-400">The Red Pill</h4>
                            </div>
                            <p className="text-[11px] text-rose-700/70 dark:text-rose-400/70 leading-relaxed font-medium">
                                Irreversible agent state purge. Wipes all Identity, Soul, Memory, and Session history.
                            </p>
                            <button
                                onClick={async () => {
                                    if (!confirm("ABSOLUTE PERMANENT RESET: Wiping your agent's soul and memory. Proceed?")) return;
                                    try {
                                        setIsLoading(true);
                                        await clawdbot.clearClawdbotMemory('all');
                                        setStatus(s => ({ ...s, gateway: 'stopped', wsConnected: false }));
                                        toast.success("Agent factory reset initiated.");
                                        setViewingFile(null);
                                    } catch (e) { toast.error("Reset failed"); }
                                    finally { setIsLoading(false); }
                                }}
                                disabled={isLoading}
                                className="w-full py-3 rounded-xl bg-rose-700 text-white font-bold hover:bg-rose-800 transition-all text-sm shadow-lg shadow-rose-900/10"
                            >
                                FACTORY RESET AGENT
                            </button>
                        </div>
                    </div>
                </div>
                {/* System Infrastructure & Diagnostics */}
                <div className="space-y-6">
                    <div className="flex items-center gap-3">
                        <Shield className="w-5 h-5 text-muted-foreground" />
                        <h4 className="text-sm font-bold uppercase tracking-[0.2em] text-muted-foreground/80">System Infrastructure</h4>
                    </div>

                    <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                        {/* OS Permissions Section (Mac Only) */}
                        {status.gatewayMode === 'local' && (
                            <div className="p-6 rounded-3xl bg-card border border-border/50 shadow-lg space-y-4">
                                <h5 className="text-xs font-bold uppercase tracking-wider text-muted-foreground">OS Governance</h5>
                                <div className="space-y-3">
                                    {[
                                        { id: 'accessibility', label: 'Accessibility', icon: MousePointerClick, granted: permissions.accessibility },
                                        { id: 'screen_recording', label: 'Vision Stream', icon: Monitor, granted: permissions.screen_recording }
                                    ].map(perm => (
                                        <div key={perm.id} className="flex items-center justify-between p-3 rounded-2xl bg-muted/20 border border-border/50">
                                            <div className="flex items-center gap-3">
                                                <perm.icon className="w-4 h-4 text-muted-foreground" />
                                                <span className="text-xs font-medium">{perm.label}</span>
                                            </div>
                                            {perm.granted ? (
                                                <div className="flex items-center gap-1.5 px-2 py-1 rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 text-[10px] font-bold">
                                                    <CheckCircle className="w-3 h-3" />
                                                    ACTIVE
                                                </div>
                                            ) : (
                                                <button
                                                    onClick={() => clawdbot.requestPermission(perm.id as any)}
                                                    className="px-3 py-1 rounded-lg bg-primary text-primary-foreground text-[10px] font-bold hover:opacity-90 transition-all"
                                                >
                                                    AUTHORIZE
                                                </button>
                                            )}
                                        </div>
                                    ))}
                                </div>
                            </div>
                        )}

                        {/* Pairing Credentials (Local Mode Only) */}
                        {status.gatewayMode === 'local' && (
                            <div className="p-6 rounded-3xl bg-card border border-border/50 shadow-lg space-y-4">
                                <h5 className="text-xs font-bold uppercase tracking-wider text-muted-foreground">Neural Pairing</h5>
                                <div className="space-y-3">
                                    <div className="space-y-1">
                                        <span className="text-[10px] font-bold text-muted-foreground/60 uppercase">Machine ID</span>
                                        <div className="flex items-center justify-between bg-muted/30 p-2.5 rounded-xl border border-border/50">
                                            <span className="text-[10px] font-mono truncate max-w-[140px]">{status.deviceId}</span>
                                            <button onClick={() => copyToClipboard(status.deviceId, 'Device ID')} className="p-1 hover:bg-primary/10 rounded-lg transition-colors">
                                                <Copy className="w-3.5 h-3.5 text-primary" />
                                            </button>
                                        </div>
                                    </div>
                                    <div className="space-y-1">
                                        <span className="text-[10px] font-bold text-muted-foreground/60 uppercase">Handshake Token</span>
                                        <div className="flex items-center justify-between bg-muted/30 p-2.5 rounded-xl border border-border/50">
                                            <span className="text-[10px] font-mono">••••••••••••••••</span>
                                            <button onClick={() => copyToClipboard(status.authToken, 'Access Token')} className="p-1 hover:bg-primary/10 rounded-lg transition-colors">
                                                <Copy className="w-3.5 h-3.5 text-primary" />
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            </div>
                        )}
                    </div>

                    {/* Infrastructure Summary & Diagnostics */}
                    <div className="p-6 rounded-3xl bg-muted/20 border border-border/50 flex flex-col md:flex-row items-center justify-between gap-6">
                        <div className="flex gap-8">
                            <div className="space-y-1">
                                <span className="text-[10px] font-bold text-muted-foreground uppercase tracking-widest">Network Bind</span>
                                <div className="flex items-center gap-2">
                                    <div className="w-1.5 h-1.5 rounded-full bg-emerald-500" />
                                    <span className="text-sm font-bold font-mono">127.0.0.1:{status.port}</span>
                                </div>
                            </div>
                            <div className="space-y-1">
                                <span className="text-[10px] font-bold text-muted-foreground uppercase tracking-widest">Environment</span>
                                <div className="flex items-center gap-2">
                                    <span className="text-sm font-bold">{isSafeMode ? 'Isolated' : 'Bridged'}</span>
                                </div>
                            </div>
                        </div>
                        <button
                            onClick={copyDiagnostics}
                            className="flex items-center gap-2 px-6 py-2.5 rounded-2xl bg-card border border-border/50 hover:bg-muted/50 transition-all text-xs font-bold uppercase tracking-wider shadow-sm"
                        >
                            <Copy className="w-4 h-4 text-primary" />
                            Copy Diagnostics Bundle
                        </button>
                    </div>
                </div>
            </div>

            {/* Context Warning Dialog */}
            {showContextWarning && (
                <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-md p-6 animate-in fade-in duration-300">
                    <motion.div
                        initial={{ opacity: 0, scale: 0.9, y: 20 }}
                        animate={{ opacity: 1, scale: 1, y: 0 }}
                        className="bg-card border border-border/50 p-8 rounded-3xl shadow-2xl max-sm w-full space-y-6 text-center"
                    >
                        <div className="mx-auto p-4 bg-amber-500/20 rounded-full w-fit">
                            <AlertTriangle className="w-8 h-8 text-amber-500" />
                        </div>
                        <div className="space-y-2">
                            <h3 className="text-xl font-bold tracking-tight">Intelligence Ceiling</h3>
                            <p className="text-sm text-muted-foreground leading-relaxed">
                                The OpenClaw engine requires <span className="text-foreground font-bold italic">32,768 tokens</span> of context to operate effectively.
                            </p>
                        </div>
                        <div className="p-4 rounded-2xl bg-muted/50 border border-border flex justify-between items-center text-xs font-bold">
                            <span className="text-muted-foreground uppercase tracking-widest">Current</span>
                            <span className="text-amber-500 underline underline-offset-4">{maxContext / 1024}k</span>
                        </div>
                        <div className="flex flex-col gap-3">
                            <button
                                onClick={() => {
                                    setMaxContext(32768);
                                    setShowContextWarning(false);
                                    toast.success("Context set to recommended minimum");
                                }}
                                className="w-full py-3 rounded-2xl bg-primary text-primary-foreground font-bold hover:opacity-90 transition-all shadow-lg shadow-primary/20"
                            >
                                Auto-Adjust to 32k
                            </button>
                            <button
                                onClick={() => {
                                    setShowContextWarning(false);
                                    executeStartGateway();
                                }}
                                className="w-full py-3 rounded-2xl bg-muted hover:bg-muted/80 text-muted-foreground font-bold transition-all"
                            >
                                Proceed with Caution
                            </button>
                        </div>
                    </motion.div>
                </div>
            )}
        </motion.div>
    );
}

function StatusBadge({ state }: { state: GatewayStatus }) {
    const config = {
        stopped: { color: 'bg-muted/30 text-muted-foreground border-border/50', icon: XCircle, label: 'HALTED', pulse: false },
        starting: { color: 'bg-amber-500/10 text-amber-600 dark:text-amber-400 border-amber-500/20', icon: RefreshCw, label: 'IGNITING', pulse: true },
        running: { color: 'bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 border-emerald-500/10', icon: CheckCircle, label: 'ORBITAL', pulse: false },
        error: { color: 'bg-rose-500/10 text-rose-600 dark:text-rose-400 border-rose-500/10', icon: AlertTriangle, label: 'SYSTEM FAILURE', pulse: true }
    }[state];

    const Icon = config.icon;

    return (
        <div className={cn(
            "flex items-center gap-2 px-4 py-2 rounded-full text-[10px] font-bold transition-all border",
            config.color,
            config.pulse && "animate-pulse"
        )}>
            <Icon className={cn("w-3.5 h-3.5", state === 'starting' && "animate-spin")} />
            {config.label}
        </div>
    );
}
