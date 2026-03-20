import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import { Radio, Play, Square, RefreshCw, Shield, AlertTriangle, CheckCircle, XCircle, Copy, Zap, Code, Monitor, Server, RotateCcw, Trash2, Globe, Cpu, Settings, FolderOpen } from 'lucide-react';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import * as openclaw from '../../lib/openclaw';
import { type CustomSecret } from '../../lib/bindings';
import { useModelContext } from '../model-context';
import { RemoteDeployWizard } from '../openclaw/RemoteDeployWizard';
import CloudBrainConfigModal from '../openclaw/CloudBrainConfigModal';

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
    allowLocalTools: boolean;
    workspaceMode: string;
    workspaceRoot: string | null;
    localInferenceEnabled: boolean;
    hasHuggingfaceToken: boolean;
    huggingfaceGranted: boolean;
    hasAnthropicKey: boolean;
    anthropicGranted: boolean;
    hasBraveKey: boolean;
    braveGranted: boolean;
    hasOpenaiKey: boolean;
    openaiGranted: boolean;
    hasOpenrouterKey: boolean;
    openrouterGranted: boolean;
    customSecrets: CustomSecret[];
    geminiGranted: boolean;
    groqGranted: boolean;
    selectedCloudBrain: string | null;
    autoStartGateway: boolean;
    profiles: openclaw.AgentProfile[];
    enabled_cloud_providers: string[];
    /** Agent runs tools without per-call approval prompts */
    autoApproveTools: boolean;
    /** First-run identity bootstrap has been completed */
    bootstrapCompleted: boolean;
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
        allowLocalTools: true,
        workspaceMode: 'sandboxed',
        workspaceRoot: null,
        localInferenceEnabled: false,
        hasHuggingfaceToken: false,
        huggingfaceGranted: false,
        hasAnthropicKey: false,
        anthropicGranted: false,
        hasBraveKey: false,
        braveGranted: false,
        hasOpenaiKey: false,
        openaiGranted: false,
        hasOpenrouterKey: false,
        openrouterGranted: false,
        customSecrets: [],
        geminiGranted: false,
        groqGranted: false,
        selectedCloudBrain: null,
        autoStartGateway: false,
        profiles: [],
        enabled_cloud_providers: [],
        autoApproveTools: false,
        bootstrapCompleted: false,
    });

    const [rawStatus, setRawStatus] = useState<openclaw.OpenClawStatus | null>(null);

    const [isCloudConfigOpen, setIsCloudConfigOpen] = useState(false); // Added this state
    const [showDeployWizard, setShowDeployWizard] = useState(false);

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
            const s = await openclaw.getOpenClawStatus();
            setStatus({
                gateway: s.engine_running ? 'running' : 'stopped',
                wsConnected: s.engine_connected,
                slackEnabled: s.slack_enabled,
                telegramEnabled: s.telegram_enabled,
                port: s.port,
                gatewayMode: s.gateway_mode,
                remoteUrl: s.remote_url,
                remoteToken: s.remote_token,
                deviceId: s.device_id,
                authToken: s.auth_token,
                stateDir: s.state_dir,
                allowLocalTools: s.allow_local_tools ?? true,
                workspaceMode: s.workspace_mode || 'unrestricted',
                workspaceRoot: s.workspace_root || null,
                localInferenceEnabled: s.local_inference_enabled,
                hasHuggingfaceToken: s.has_huggingface_token,
                huggingfaceGranted: s.huggingface_granted,
                hasAnthropicKey: s.has_anthropic_key,
                anthropicGranted: s.anthropic_granted,
                hasBraveKey: s.has_brave_key,
                braveGranted: s.brave_granted,
                hasOpenaiKey: s.has_openai_key,
                openaiGranted: s.openai_granted,
                hasOpenrouterKey: s.has_openrouter_key,
                openrouterGranted: s.openrouter_granted,
                customSecrets: s.custom_secrets || [],
                geminiGranted: s.gemini_granted,
                groqGranted: s.groq_granted,
                selectedCloudBrain: s.selected_cloud_brain,
                autoStartGateway: s.auto_start_gateway || false,
                profiles: s.profiles || [],
                enabled_cloud_providers: s.enabled_cloud_providers || [],
                autoApproveTools: s.auto_approve_tools || false,
                bootstrapCompleted: s.bootstrap_completed || false,
            });
            setRawStatus(s);

            const perms = await openclaw.getPermissionStatus();
            setPermissions(perms);

            if (s.remote_url) setRemoteUrlInput(s.remote_url);
            if (s.remote_token) setRemoteTokenInput(s.remote_token);
        } catch (e) {
            console.error('Failed to fetch openclaw status:', e);
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
            await openclaw.startOpenClawGateway();
            await fetchStatus();
            toast.success('OpenClaw Gateway started');
        } catch (e) {
            console.error('Failed to start gateway:', e);
            setStatus(s => ({ ...s, gateway: 'error' }));
            toast.error('Failed to start gateway', { description: String(e) });
        } finally {
            setIsLoading(false);
        }
    };

    const handleStart = async () => {
        // Mandatory Inference Provider Check
        const cloudGranted = status.anthropicGranted || status.openaiGranted || status.openrouterGranted || status.geminiGranted || status.groqGranted || (status.customSecrets && status.customSecrets.some(s => s.granted)); // Also check custom secrets if any relate to LLMs
        if (!status.localInferenceEnabled && !cloudGranted) {
            toast.error("Cognitive engine required", {
                description: "Please enable Local Neural Link or authorize a Cloud Brain before starting the gateway."
            });
            return;
        }

        if (status.gatewayMode === 'local' && maxContext < 32768) {
            setShowContextWarning(true);
            return;
        }
        await executeStartGateway();
    };

    const handleStop = async () => {
        setIsLoading(true);

        try {
            await openclaw.stopOpenClawGateway();
            await fetchStatus();
            toast.info('OpenClaw Gateway stopped');
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
            await openclaw.saveGatewaySettings(mode, url, token);
            await fetchStatus();
            toast.success('Gateway settings updated');
        } catch (e) {
            toast.error('Failed to update gateway settings', { description: String(e) });
        }
    };

    const copyToClipboard = (text: string, label: string = "Text") => {
        navigator.clipboard.writeText(text);
        toast.success(`${label} copied to clipboard`);
    };

    const copyDiagnostics = async () => {
        try {
            const diag = await openclaw.getOpenClawDiagnostics();
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
                        <h4 className="text-xl font-bold tracking-tight">OpenClawEngine Sync</h4>
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
        <div className="space-y-4">
            <div className="grid grid-cols-1 gap-3">
                {/* Always show Local Core */}
                <button
                    onClick={() => handleSaveGateway('local', null, null)}
                    className={cn(
                        "p-4 rounded-xl text-left border transition-all flex items-center justify-between group",
                        status.gatewayMode === 'local'
                            ? "bg-primary/5 border-primary/40 shadow-sm"
                            : "bg-card border-border/50 hover:bg-muted/50 hover:border-primary/30"
                    )}
                >
                    <div className="flex items-center gap-4">
                        <div className="p-2.5 rounded-lg bg-primary/10 text-primary">
                            <Monitor className="w-5 h-5" />
                        </div>
                        <div>
                            <div className="font-bold text-sm">Local Core</div>
                            <div className="text-[10px] text-muted-foreground font-medium uppercase tracking-wide">
                                Internal Host • 127.0.0.1
                            </div>
                            {/* macOS Warning */}
                            {navigator.platform.toUpperCase().includes('MAC') && (
                                <div className="mt-1 flex items-center gap-1.5 text-[10px] text-rose-500 font-bold bg-rose-500/10 px-2 py-0.5 rounded border border-rose-500/20">
                                    <AlertTriangle className="w-3 h-3" />
                                    <span>High Risk: Unrestricted Root Access</span>
                                </div>
                            )}
                        </div>
                    </div>
                    {status.gatewayMode === 'local' && (
                        <div className="flex items-center gap-2 text-emerald-500 text-xs font-bold bg-emerald-500/10 px-2 py-1 rounded-md">
                            <div className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
                            ACTIVE
                        </div>
                    )}
                </button>

                {/* Remote Profiles */}
                {status.profiles.map((profile) => (
                    <div
                        key={profile.id}
                        className={cn(
                            "p-4 rounded-xl text-left border transition-all flex items-center justify-between group",
                            status.gatewayMode === 'remote' && status.remoteUrl === profile.url
                                ? "bg-indigo-500/5 border-indigo-500/40 shadow-sm"
                                : "bg-card border-border/50 hover:bg-muted/50 hover:border-indigo-500/30"
                        )}
                    >
                        <button
                            onClick={() => handleSaveGateway('remote', profile.url, profile.token)}
                            className="flex items-center gap-4 flex-1 text-left"
                        >
                            <div className="p-2.5 rounded-lg bg-indigo-500/10 text-indigo-500">
                                <Server className="w-5 h-5" />
                            </div>
                            <div>
                                <div className="font-bold text-sm">{profile.name}</div>
                                <div className="text-[10px] text-muted-foreground font-medium uppercase tracking-wide font-mono">
                                    {profile.url.replace('ws://', '').replace('wss://', '')}
                                </div>
                            </div>
                        </button>

                        <div className="flex items-center gap-2">
                            {status.gatewayMode === 'remote' && status.remoteUrl === profile.url ? (
                                <div className="flex items-center gap-2 text-indigo-500 text-xs font-bold bg-indigo-500/10 px-2 py-1 rounded-md">
                                    <div className="w-1.5 h-1.5 rounded-full bg-indigo-500 animate-pulse" />
                                    CONNECTED
                                </div>
                            ) : (
                                <button
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        openclaw.removeAgentProfile(profile.id).then(fetchStatus);
                                    }}
                                    className="p-2 text-muted-foreground hover:text-rose-500 hover:bg-rose-500/10 rounded-lg transition-colors"
                                    title="Remove Profile"
                                >
                                    <Trash2 className="w-4 h-4" />
                                </button>
                            )}
                        </div>
                    </div>
                ))}
            </div>

            <button
                onClick={() => setShowDeployWizard(true)}
                className="w-full py-3 rounded-xl border border-dashed border-border hover:border-primary/50 text-muted-foreground hover:text-primary hover:bg-primary/5 transition-all flex items-center justify-center gap-2 text-sm font-bold uppercase tracking-wide"
            >
                <Server className="w-4 h-4" />
                Add New Agent
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
                            "flex items-center justify-center gap-2 bg-card border border-border/50 group",
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
                            <h2 className="text-2xl font-bold tracking-tight">OpenClaw Gateway</h2>
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
                        <div className="flex items-center justify-between border-b border-border/50 pb-4">
                            <div className="flex items-center gap-3">
                                <Server className="w-6 h-6 text-indigo-500" />
                                <div>
                                    <h4 className="font-bold text-lg">Remote Bridge Connection</h4>
                                    <p className="text-xs text-muted-foreground">Linking your desktop client to an external OpenClawEngine instance.</p>
                                </div>
                            </div>
                            <button
                                onClick={() => setShowDeployWizard(true)}
                                className="px-3 py-1.5 rounded-lg bg-indigo-500/10 hover:bg-indigo-500/20 text-indigo-400 text-xs font-bold border border-indigo-500/20 transition-all flex items-center gap-2"
                            >
                                <Server className="w-3.5 h-3.5" />
                                DEPLOY NEW SERVER
                            </button>
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
                    <div className="flex items-center justify-between border-b border-border/50 pb-4">
                        <div className="flex items-center gap-3">
                            <Monitor className="w-6 h-6 text-amber-500" />
                            <div>
                                <h4 className="font-bold text-lg">Agent Tuning</h4>
                                <p className="text-xs text-muted-foreground">Configuration for autonomous behavior and intelligence.</p>
                            </div>
                        </div>
                        <div className={cn(
                            "px-3 py-1 rounded-lg text-[10px] font-bold border flex items-center gap-2",
                            status.localInferenceEnabled
                                ? "bg-emerald-500/10 text-emerald-600 border-emerald-500/20"
                                : "bg-amber-500/10 text-amber-600 border-amber-500/20"
                        )}>
                            <div className={cn("w-1.5 h-1.5 rounded-full animate-pulse", status.localInferenceEnabled ? "bg-emerald-500" : "bg-amber-500")} />
                            {status.localInferenceEnabled ? "NEURAL LINK ACTIVE" : "CLOUD BRAIN ACTIVE"}
                        </div>
                    </div>

                    <div className="bg-amber-500/5 border border-amber-500/20 p-4 rounded-xl flex gap-4">
                        <AlertTriangle className="w-8 h-8 text-amber-500 mt-1 shrink-0" />
                        <p className="text-sm text-amber-700 dark:text-amber-400 leading-relaxed">
                            <span className="font-bold">Intelligence Requirement:</span> Agents function optimally with a <span className="underline">32k minimum context window</span>. Verify your memory settings if you experience session fragmentation.
                        </p>
                    </div>

                    {/* Intelligence Channels Redesign */}
                    <div className="space-y-4">
                        <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Primary Intelligence Source</label>

                        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                            {/* Local Core Card */}
                            <button
                                onClick={async () => {
                                    if (!status.localInferenceEnabled) {
                                        await openclaw.toggleOpenClawLocalInference(true);
                                        await fetchStatus();
                                        toast.success('Switched to Local Core');
                                    }
                                }}
                                className={cn(
                                    "relative p-5 rounded-2xl border-2 text-left transition-all duration-300 overflow-hidden group",
                                    status.localInferenceEnabled
                                        ? "bg-emerald-500/10 border-emerald-500/50 shadow-lg shadow-emerald-500/20"
                                        : "bg-card border-border hover:border-emerald-500/30 hover:bg-emerald-500/5"
                                )}
                            >
                                <div className="flex items-start justify-between mb-4">
                                    <div className={cn(
                                        "p-3 rounded-xl transition-colors",
                                        status.localInferenceEnabled ? "bg-emerald-500 text-white" : "bg-muted text-muted-foreground group-hover:text-emerald-500"
                                    )}>
                                        <Cpu className="w-6 h-6" />
                                    </div>
                                    {status.localInferenceEnabled && (
                                        <div className="px-2 py-1 rounded-full bg-emerald-500 text-white text-[10px] font-bold uppercase tracking-wider">
                                            Active
                                        </div>
                                    )}
                                </div>
                                <h3 className="text-lg font-bold mb-1">Local Core</h3>
                                <p className="text-xs text-muted-foreground mb-4">
                                    Privacy-first inference on this device. Zero data egress. Also powers connected remote agents.
                                </p>
                                <div className="flex items-center gap-2 text-[10px] font-medium text-emerald-600/80">
                                    <div className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
                                    {status.localInferenceEnabled ? "Neural Engine Online" : "Ready to Activate"}
                                </div>
                            </button>

                            {/* Cloud Intelligence Card */}
                            <div className={cn(
                                "relative p-5 rounded-2xl border-2 text-left transition-all duration-300 overflow-hidden group flex flex-col",
                                !status.localInferenceEnabled
                                    ? "bg-indigo-500/10 border-indigo-500/50 shadow-lg shadow-indigo-500/20"
                                    : "bg-card border-border hover:border-indigo-500/30 hover:bg-indigo-500/5"
                            )}>
                                <button
                                    className="absolute inset-0 z-0"
                                    onClick={async () => {
                                        if (status.localInferenceEnabled) {
                                            await openclaw.toggleOpenClawLocalInference(false);
                                            await fetchStatus();
                                            toast.success('Switched to Cloud Intelligence');
                                        }
                                    }}
                                />

                                <div className="relative z-10 flex items-start justify-between mb-4">
                                    <div className={cn(
                                        "p-3 rounded-xl transition-colors",
                                        !status.localInferenceEnabled ? "bg-indigo-500 text-white" : "bg-muted text-muted-foreground group-hover:text-indigo-500"
                                    )}>
                                        <Globe className="w-6 h-6" />
                                    </div>
                                    <div className="flex items-center gap-2">
                                        {!status.localInferenceEnabled && (
                                            <div className="px-2 py-1 rounded-full bg-indigo-500 text-white text-[10px] font-bold uppercase tracking-wider">
                                                Active
                                            </div>
                                        )}
                                    </div>
                                </div>

                                <h3 className="text-lg font-bold mb-1">Cloud Intelligence</h3>
                                <p className="text-xs text-muted-foreground mb-4">
                                    High-performance models from top providers. Best for complex reasoning.
                                </p>

                                <div className="mt-auto relative z-10 pt-2 border-t border-indigo-500/10 flex items-center justify-between">
                                    <div className="flex flex-col">
                                        <span className="text-[10px] font-bold uppercase text-muted-foreground">Enabled Providers</span>
                                        <div className="flex items-center gap-1 mt-1 h-5">
                                            {status.enabled_cloud_providers?.length > 0 ? (
                                                <span className="text-xs font-medium truncate max-w-[120px]">
                                                    {status.enabled_cloud_providers.join(', ')}
                                                </span>
                                            ) : (
                                                <span className="text-xs text-muted-foreground italic">None configured</span>
                                            )}
                                        </div>
                                    </div>

                                    <button
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            setIsCloudConfigOpen(true);
                                        }}
                                        className="px-3 py-1.5 rounded-lg bg-background/50 hover:bg-background border border-border text-xs font-bold flex items-center gap-2 transition-colors"
                                    >
                                        <Settings className="w-3 h-3" />
                                        Configure
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>
                </div>

                {/* Autonomy Mode + Identity Ritual */}
                <div className="p-8 rounded-3xl bg-card border border-border/50 shadow-xl space-y-6">
                    <div className="flex items-center gap-3 border-b border-border/50 pb-4">
                        <Zap className="w-6 h-6 text-violet-500" />
                        <div>
                            <h4 className="font-bold text-lg">Agency & Identity</h4>
                            <p className="text-xs text-muted-foreground">Control how autonomously the agent acts, and manage its identity ritual.</p>
                        </div>
                    </div>

                    {/* Autonomy toggle */}
                    <div className="flex items-center justify-between py-2">
                        <div className="flex flex-col gap-0.5">
                            <span className="text-sm font-bold">Fully Autonomous Mode</span>
                            <span className="text-xs text-muted-foreground">
                                {status.autoApproveTools
                                    ? 'Agent runs tools without per-call approval — maximum autonomy.'
                                    : 'You approve each tool call before it runs — human-in-the-loop.'}
                            </span>
                        </div>
                        <button
                            onClick={async () => {
                                const next = !status.autoApproveTools;
                                try {
                                    await openclaw.setAutonomyMode(next);
                                    setStatus(s => ({ ...s, autoApproveTools: next }));
                                    toast.success(next ? 'Autonomous mode enabled — restart gateway' : 'Human-in-the-loop mode enabled');
                                } catch (e) { toast.error('Failed to update autonomy mode'); }
                            }}
                            className={cn(
                                'relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-all duration-300',
                                status.autoApproveTools ? 'bg-violet-600' : 'bg-muted'
                            )}
                        >
                            <span className={cn(
                                'inline-block h-5 w-5 rounded-full bg-white shadow-lg transition-transform duration-300',
                                status.autoApproveTools ? 'translate-x-5' : 'translate-x-0'
                            )} />
                        </button>
                    </div>

                    {status.autoApproveTools && (
                        <div className="bg-violet-500/5 border border-violet-500/20 p-3 rounded-xl flex gap-3 text-xs text-violet-700 dark:text-violet-300">
                            <Zap className="w-4 h-4 shrink-0 mt-0.5" />
                            <span>The agent will use tools freely without asking for approval. Restart the gateway for this change to take effect.</span>
                        </div>
                    )}

                    {/* Separator */}
                    <div className="border-t border-border/50" />

                    {/* Identity Ritual */}
                    <div className="flex items-center justify-between">
                        <div className="flex flex-col gap-0.5">
                            <span className="text-sm font-bold">Identity Ritual</span>
                            <span className="text-xs text-muted-foreground">
                                {status.bootstrapCompleted
                                    ? 'Bootstrap complete. Re-initiate to restart the identity awakening dialogue.'
                                    : 'First-run ritual pending. The agent will introduce itself on next chat.'}
                            </span>
                        </div>
                        <button
                            onClick={async () => {
                                try {
                                    await openclaw.triggerBootstrap();
                                    setStatus(s => ({ ...s, bootstrapCompleted: false }));
                                    toast.success('Identity ritual re-initiated — start a new chat!');
                                } catch (e) { toast.error('Failed to trigger bootstrap'); }
                            }}
                            className="px-4 py-2 rounded-xl bg-violet-500/10 hover:bg-violet-500/20 border border-violet-500/20 text-violet-600 dark:text-violet-400 text-xs font-bold transition-all flex items-center gap-2"
                        >
                            <RotateCcw className="w-3.5 h-3.5" />
                            {status.bootstrapCompleted ? 'Reinitiate' : 'Pending…'}
                        </button>
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
                                        await openclaw.revealPath(`${baseDir}/workspace`);
                                    } catch (e) { toast.error('Directory access denied'); }
                                }
                            }}
                            className="px-5 py-2 rounded-xl bg-primary/10 hover:bg-primary/20 text-xs font-bold transition-all text-primary border border-primary/20"
                        >
                            Reveal Workspace
                        </button >
                    </div >

                    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                        {[
                            { id: 'IDENTITY.md', label: 'Identity', icon: Shield },
                            { id: 'SOUL.md', label: 'Soul', icon: Zap },
                            { id: 'MEMORY.md', label: 'Chronicles', icon: RotateCcw, memory: true },
                            { id: 'USER.md', label: 'Observer', icon: Monitor },
                            { id: 'AGENTS.md', label: 'Agents', icon: Code },
                            { id: 'TOOLS.md', label: 'Tools', icon: Settings },
                            { id: 'BOOT.md', label: 'Boot Hook', icon: Play },
                            { id: 'HEARTBEAT.md', label: 'Heartbeat', icon: RefreshCw },
                        ].map(file => (
                            <button
                                key={file.id}
                                onClick={async () => {
                                    try {
                                        const content = file.memory
                                            ? await openclaw.getOpenClawMemory()
                                            : await openclaw.getOpenClawFile(file.id);
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

                    {
                        viewingFile && (
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
                        )
                    }
                </div >


                <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                    {/* Local Dev Tools Card */}
                    <div className="p-6 rounded-3xl bg-card border border-border/50 shadow-xl space-y-4 group">
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-3">
                                <div className="p-2 bg-amber-500/10 rounded-xl group-hover:bg-amber-500/20 transition-colors">
                                    <Code className="w-5 h-5 text-amber-500" />
                                </div>
                                <h4 className="font-bold text-base">Dev Tools</h4>
                            </div>
                            <button
                                onClick={async () => {
                                    try {
                                        await openclaw.toggleOpenClawLocalTools(!status.allowLocalTools);
                                        await fetchStatus();
                                        toast.success(status.allowLocalTools ? 'Dev tools disabled' : 'Dev tools enabled');
                                    } catch (e) { toast.error('toggle failed'); }
                                }}
                                className={cn(
                                    "relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-all duration-300",
                                    status.allowLocalTools ? "bg-amber-600" : "bg-muted"
                                )}
                            >
                                <span className={cn(
                                    "inline-block h-5 w-5 rounded-full bg-white shadow-lg transition-transform duration-300",
                                    status.allowLocalTools ? "translate-x-5" : "translate-x-0"
                                )} />
                            </button>
                        </div>
                        <p className="text-xs text-muted-foreground leading-relaxed">
                            Allow the agent to read/write files, run shell commands, and execute code. Requires engine restart.
                        </p>
                    </div>

                    {/* Persistent Engine Card */}
                    <div className="p-6 rounded-3xl bg-card border border-border/50 shadow-xl space-y-4 group">
                        <div className="flex items-center justify-between">
                            <div className="flex items-center gap-3">
                                <div className="p-2 bg-blue-500/10 rounded-xl group-hover:bg-blue-500/20 transition-colors">
                                    <RotateCcw className="w-5 h-5 text-blue-500" />
                                </div>
                                <h4 className="font-bold text-base">Persistent Engine</h4>
                            </div>
                            <button
                                onClick={async () => {
                                    try {
                                        await openclaw.toggleOpenClawAutoStart(!status.autoStartGateway);
                                        await fetchStatus();
                                        toast.success(`Auto-start ${!status.autoStartGateway ? 'enabled' : 'disabled'}`);
                                    } catch (e) { toast.error('Toggle failed'); }
                                }}
                                className={cn(
                                    "relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-all duration-300",
                                    status.autoStartGateway ? "bg-blue-600" : "bg-muted"
                                )}
                            >
                                <span className={cn(
                                    "inline-block h-5 w-5 rounded-full bg-white shadow-lg transition-transform duration-300",
                                    status.autoStartGateway ? "translate-x-5" : "translate-x-0"
                                )} />
                            </button>
                        </div>
                        <p className="text-xs text-muted-foreground leading-relaxed">
                            Automatically engage the gateway on application launch for immediate agent availability.
                        </p>
                    </div>

                    {/* Workspace Mode Card */}
                    <div className="p-6 rounded-3xl bg-card border border-border/50 shadow-xl space-y-4 group md:col-span-2">
                        <div className="flex items-center gap-3 mb-2">
                            <div className="p-2 bg-violet-500/10 rounded-xl group-hover:bg-violet-500/20 transition-colors">
                                <FolderOpen className="w-5 h-5 text-violet-500" />
                            </div>
                            <div>
                                <h4 className="font-bold text-base">Workspace Mode</h4>
                                <p className="text-xs text-muted-foreground">Controls where the agent can read/write files. Requires engine restart.</p>
                            </div>
                        </div>

                        <div className="grid grid-cols-3 gap-3">
                            {([
                                {
                                    mode: 'sandboxed',
                                    label: 'Sandboxed',
                                    icon: '🔒',
                                    desc: 'Confined to workspace. Files outside are blocked entirely.',
                                    borderColor: '#f59e0b',
                                    bgColor: 'rgba(245,158,11,0.1)',
                                },
                                {
                                    mode: 'project',
                                    label: 'Project',
                                    icon: '📁',
                                    desc: 'Working directory set. Can still access other files if needed.',
                                    borderColor: '#3b82f6',
                                    bgColor: 'rgba(59,130,246,0.1)',
                                },
                                {
                                    mode: 'unrestricted',
                                    label: 'Unrestricted',
                                    icon: '🌐',
                                    desc: 'Full system access. Agent can access any file with your approval.',
                                    borderColor: '#10b981',
                                    bgColor: 'rgba(16,185,129,0.1)',
                                },
                            ] as const).map(({ mode, label, icon, desc, borderColor, bgColor }) => (
                                <button
                                    key={mode}
                                    onClick={async () => {
                                        try {
                                            // Pass null for root — backend will auto-generate if needed
                                            const resolvedPath = await openclaw.setOpenClawWorkspaceMode(
                                                mode,
                                                mode === 'unrestricted' ? null : (status.workspaceRoot || null)
                                            );
                                            setStatus(prev => ({
                                                ...prev,
                                                workspaceMode: mode,
                                                workspaceRoot: resolvedPath === 'none' ? null : resolvedPath,
                                            }));
                                            toast.success(`Workspace mode: ${label}`);
                                        } catch (e) {
                                            toast.error(e?.toString() || 'Failed to set workspace mode');
                                        }
                                    }}
                                    className={cn(
                                        "relative p-4 rounded-xl border-2 text-left transition-all duration-200",
                                        status.workspaceMode === mode
                                            ? "shadow-lg"
                                            : "border-border/50 hover:border-border hover:bg-muted/30"
                                    )}
                                    style={status.workspaceMode === mode ? {
                                        borderColor,
                                        backgroundColor: bgColor,
                                    } : undefined}
                                >
                                    <div className="flex items-center gap-2 mb-1.5">
                                        <span className="text-lg">{icon}</span>
                                        <span className="font-semibold text-sm">{label}</span>
                                    </div>
                                    <div className="text-[10px] text-muted-foreground leading-tight">{desc}</div>
                                    {status.workspaceMode === mode && (
                                        <div className="absolute top-2.5 right-2.5">
                                            <CheckCircle className="w-4 h-4" style={{ color: borderColor }} />
                                        </div>
                                    )}
                                </button>
                            ))}
                        </div>

                        {/* Workspace path display & editor (shown for sandboxed/project modes) */}
                        {status.workspaceMode !== 'unrestricted' && (
                            <div className="space-y-2 pt-1">
                                {status.workspaceRoot && (
                                    <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-muted/30 border border-border/30">
                                        <FolderOpen className="w-4 h-4 text-muted-foreground shrink-0" />
                                        <span className="text-xs text-muted-foreground font-mono truncate flex-1" title={status.workspaceRoot}>
                                            {status.workspaceRoot}
                                        </span>
                                        <span className="text-[10px] text-muted-foreground/60 shrink-0">
                                            {status.workspaceMode === 'sandboxed' ? 'sandboxed' : 'working dir'}
                                        </span>
                                    </div>
                                )}
                                <div className="flex items-center gap-2">
                                    <input
                                        type="text"
                                        value={status.workspaceRoot || ''}
                                        onChange={(e) => setStatus(prev => ({ ...prev, workspaceRoot: e.target.value || null }))}
                                        placeholder="Custom path (leave empty for default)"
                                        className="flex-1 px-3 py-2 rounded-lg bg-muted/50 border border-border/50 text-xs font-mono placeholder:text-muted-foreground/40 focus:outline-none focus:ring-1 focus:ring-violet-500/50"
                                    />
                                    <button
                                        onClick={async () => {
                                            try {
                                                const resolvedPath = await openclaw.setOpenClawWorkspaceMode(
                                                    status.workspaceMode,
                                                    status.workspaceRoot
                                                );
                                                setStatus(prev => ({
                                                    ...prev,
                                                    workspaceRoot: resolvedPath === 'none' ? null : resolvedPath,
                                                }));
                                                toast.success('Workspace directory updated');
                                            } catch (e) {
                                                toast.error(e?.toString() || 'Failed to set workspace root');
                                            }
                                        }}
                                        className="px-4 py-2 rounded-lg bg-violet-500/10 hover:bg-violet-500/20 text-violet-500 text-xs font-semibold transition-colors whitespace-nowrap"
                                    >
                                        Apply
                                    </button>
                                </div>
                                <p className="text-[10px] text-muted-foreground/60 leading-relaxed">
                                    Leave empty to use the default workspace inside the app data directory. Or enter a custom path.
                                </p>
                            </div>
                        )}
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
                        <p className="text-[11px] text-emerald-600/70 leading-relaxed font-medium">
                            Pure loopback binding (127.0.0.1) enforced. Token authentication required for all IPC/WS requests. Discovery disabled.
                        </p>
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
                                    // Removed confirm dialog to prevent browser blocking; this action is deliberate enough
                                    try {
                                        setIsLoading(true);
                                        // Force stop gateway first to ensure clean state
                                        await openclaw.stopOpenClawGateway();
                                        await openclaw.clearOpenClawMemory('all');
                                        setStatus(s => ({ ...s, gateway: 'stopped', wsConnected: false }));
                                        toast.success("Agent factory reset initiated.");
                                        setViewingFile(null);
                                    } catch (e) {
                                        console.error(e);
                                        toast.error("Reset failed", { description: String(e) });
                                    }
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
                                <div className="flex items-center justify-between">
                                    <h5 className="text-xs font-bold uppercase tracking-wider text-muted-foreground">OS Governance</h5>
                                    <button
                                        onClick={async () => {
                                            const perms = await openclaw.getPermissionStatus();
                                            setPermissions(perms);
                                        }}
                                        className="text-[9px] text-muted-foreground/50 hover:text-muted-foreground transition-colors uppercase tracking-wider font-bold"
                                    >
                                        Refresh
                                    </button>
                                </div>
                                <div className="space-y-3">
                                    {[
                                        { id: 'screen_recording', label: 'Vision Stream', desc: 'Allows the agent to capture screenshots for visual context. Requires Dev Tools to be enabled.', icon: Monitor, granted: permissions.screen_recording }
                                    ].map(perm => (
                                        <div key={perm.id} className="flex items-center justify-between p-3.5 rounded-2xl bg-muted/20 border border-border/50 transition-all duration-300">
                                            <div className="flex items-center gap-3">
                                                <div className={cn(
                                                    "p-1.5 rounded-lg transition-colors",
                                                    perm.granted ? "bg-emerald-500/10" : "bg-muted/50"
                                                )}>
                                                    <perm.icon className={cn("w-4 h-4", perm.granted ? "text-emerald-500" : "text-muted-foreground")} />
                                                </div>
                                                <div>
                                                    <span className="text-xs font-semibold block">{perm.label}</span>
                                                    <span className="text-[10px] text-muted-foreground/60">{perm.desc}</span>
                                                </div>
                                            </div>
                                            {perm.granted ? (
                                                <div className="flex items-center gap-2">
                                                    <div className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 text-[10px] font-bold">
                                                        <CheckCircle className="w-3 h-3" />
                                                        ACTIVE
                                                    </div>
                                                    <button
                                                        onClick={async () => {
                                                            await openclaw.openPermissionSettings(perm.id);
                                                            toast.info(`Revoke ${perm.label} in System Settings`, {
                                                                description: "Toggle the switch off, then restart Scrappy for changes to take effect."
                                                            });
                                                            let checks = 0;
                                                            const poller = setInterval(async () => {
                                                                checks++;
                                                                if (checks > 15) { clearInterval(poller); return; }
                                                                const fresh = await openclaw.getPermissionStatus();
                                                                setPermissions(fresh);
                                                                if (!fresh[perm.id as keyof typeof fresh]) {
                                                                    toast.success(`${perm.label} revoked. Restart Scrappy to apply.`);
                                                                    clearInterval(poller);
                                                                }
                                                            }, 2000);
                                                        }}
                                                        className="px-2 py-1 rounded-lg border border-border/50 text-[10px] font-bold text-muted-foreground hover:text-foreground hover:bg-muted/50 active:scale-95 transition-all"
                                                        title="Open System Settings to revoke"
                                                    >
                                                        Manage
                                                    </button>
                                                </div>
                                            ) : (
                                                <button
                                                    onClick={async () => {
                                                        try {
                                                            const updated = await openclaw.requestPermission(perm.id);
                                                            setPermissions(updated);
                                                            if (updated[perm.id as keyof typeof updated]) {
                                                                toast.success(`${perm.label} permission granted`);
                                                            } else {
                                                                toast.info(`Grant ${perm.label} in System Settings, then return here`, {
                                                                    description: "System Settings should have opened. After granting access, the status updates automatically."
                                                                });
                                                                let checks = 0;
                                                                const poller = setInterval(async () => {
                                                                    checks++;
                                                                    if (checks > 15) { clearInterval(poller); return; }
                                                                    const fresh = await openclaw.getPermissionStatus();
                                                                    setPermissions(fresh);
                                                                    if (fresh[perm.id as keyof typeof fresh]) {
                                                                        toast.success(`${perm.label} permission granted!`);
                                                                        clearInterval(poller);
                                                                    }
                                                                }, 2000);
                                                            }
                                                        } catch (e) {
                                                            toast.error(`Failed to request ${perm.label}`, { description: String(e) });
                                                        }
                                                    }}
                                                    className="px-3.5 py-1.5 rounded-lg bg-primary text-primary-foreground text-[10px] font-bold hover:opacity-90 active:scale-95 transition-all shadow-sm"
                                                >
                                                    AUTHORIZE
                                                </button>
                                            )}
                                        </div>
                                    ))}
                                </div>
                                <p className="text-[10px] text-muted-foreground/40 leading-relaxed">
                                    macOS manages this permission at the system level. Restart Scrappy after changing for it to take effect.
                                </p>
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
            </div >

            {/* Context Warning Dialog */}
            {
                showContextWarning && (
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
                )
            }


            {/* Modals */}
            <CloudBrainConfigModal
                isOpen={isCloudConfigOpen}
                onClose={() => setIsCloudConfigOpen(false)}
                status={rawStatus}
                onUpdate={fetchStatus}
            />

            <RemoteDeployWizard
                isOpen={showDeployWizard}
                onClose={() => setShowDeployWizard(false)}
                onCheckStatus={() => {
                    setShowDeployWizard(false);
                    fetchStatus();
                }}
            />
        </motion.div >
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
