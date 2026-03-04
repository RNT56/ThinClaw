import { useState, useEffect, useCallback, useRef } from 'react';
import {
    MessageCircle, Radio, ChevronLeft, RefreshCw, Settings,
    Layout, Smartphone, Timer, Package, Cpu, Shield, Brain, History,
    ChevronDown, Server, Laptop, Trash2, Anchor, Plug, Settings2, Activity,
    Stethoscope, Wrench, KeyRound, DollarSign, Database, Zap, FileText, Star, GitBranch
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { OpenClawSession } from '../../lib/openclaw';

import { motion } from 'framer-motion';

export type OpenClawPage = 'chat' | 'dashboard' | 'fleet' | 'channels' | 'channel-status' | 'presence' | 'automations' | 'routine-audit' | 'skills' | 'hooks' | 'plugins' | 'system-control' | 'brain' | 'memory' | 'config' | 'event-inspector' | 'doctor' | 'tool-policies' | 'pairing' | 'cost-dashboard' | 'cache-stats' | 'routing';

const containerVariants = {
    hidden: { opacity: 0 },
    visible: {
        opacity: 1,
        transition: {
            staggerChildren: 0.05,
        }
    }
};

const itemVariants = {
    hidden: { opacity: 0 },
    visible: { opacity: 1 },
    exit: { opacity: 0 }
};

interface OpenClawSidebarProps {
    sidebarOpen: boolean;
    onBack: () => void;
    onSelectSession: (sessionKey: string) => void;
    onNewSession: () => void;
    selectedSessionKey: string | null;
    gatewayRunning: boolean;
    onNavigateToSettings: (page: 'openclaw-gateway') => void;
    activePage: OpenClawPage;
    onSelectPage: (page: OpenClawPage) => void;
}

export function OpenClawSidebar({
    sidebarOpen,
    onBack,
    onSelectSession,
    onNewSession,
    selectedSessionKey,
    gatewayRunning,
    onNavigateToSettings,
    activePage,
    onSelectPage
}: OpenClawSidebarProps) {
    const [sessions, setSessions] = useState<OpenClawSession[]>([]);
    const [isLoading, setIsLoading] = useState(false);

    const [status, setStatus] = useState<openclaw.OpenClawStatus | null>(null);
    const [isAgentListOpen, setIsAgentListOpen] = useState(false);

    const fetchData = useCallback(async () => {
        // Always fetch status to know if gateway is running
        try {
            const s = await openclaw.getOpenClawStatus();
            setStatus(s);

            if (s.gateway_running && activePage === 'chat') {
                const res = await openclaw.getOpenClawSessions();
                setSessions(res.sessions);
            } else if (!s.gateway_running) {
                // Clear stale sessions when gateway is down (e.g. after factory reset)
                setSessions([]);
            }
        } catch (e) {
            console.error('Failed to fetch data:', e);
            // If we can't reach the gateway, clear sessions
            setSessions([]);
        } finally {
            setIsLoading(false);
        }
    }, [activePage]);

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 5000); // Polling status every 5s
        return () => clearInterval(interval);
    }, [fetchData]);

    // Track which session is pending delete confirmation
    const [pendingDeleteKey, setPendingDeleteKey] = useState<string | null>(null);
    const deleteTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const handleDeleteSession = async (e: React.MouseEvent, sessionKey: string) => {
        e.stopPropagation();
        e.preventDefault();

        // Two-click confirmation: first click shows "confirm?", second click deletes
        if (pendingDeleteKey !== sessionKey) {
            setPendingDeleteKey(sessionKey);
            // Auto-dismiss after 3 seconds
            if (deleteTimeoutRef.current) clearTimeout(deleteTimeoutRef.current);
            deleteTimeoutRef.current = setTimeout(() => setPendingDeleteKey(null), 3000);
            return;
        }

        // Second click — confirmed, proceed with delete
        setPendingDeleteKey(null);
        if (deleteTimeoutRef.current) clearTimeout(deleteTimeoutRef.current);

        const tId = toast.loading("Deleting session...");

        // Attempt to abort first, just in case
        try {
            await openclaw.abortOpenClawChat(sessionKey);
        } catch (ignored) {
            // Ignore abort failure, proceed to delete
        }

        try {
            await openclaw.deleteOpenClawSession(sessionKey);
            toast.success("Session deleted", { id: tId });
            // Immediately remove from local state for instant UI feedback
            setSessions(prev => prev.filter(s => s.session_key !== sessionKey));
            fetchData(); // Also refresh from server
            if (selectedSessionKey === sessionKey) {
                onSelectSession('agent:main'); // fallback
            }
        } catch (err: any) {
            console.error("Delete session failed", err);
            const msg = err?.message || String(err);

            // If it fails because it's active, try force delete automatically
            if (msg.includes("still active") || msg.includes("timeout")) {
                try {
                    toast.loading("Session active, force deleting...", { id: tId });
                    await openclaw.resetOpenClawSession(sessionKey);
                    // After reset, try delete again to remove empty session
                    await openclaw.deleteOpenClawSession(sessionKey);
                    toast.success("Session force deleted", { id: tId });
                    setSessions(prev => prev.filter(s => s.session_key !== sessionKey));
                    fetchData();
                    if (selectedSessionKey === sessionKey) {
                        onSelectSession('agent:main');
                    }
                } catch (e2) {
                    toast.error("Force delete failed: " + String(e2), { id: tId });
                }
            } else {
                toast.error(`Failed to delete session: ${msg}`, { id: tId });
            }
        }
    };

    const handleSwitchAgent = async (profile: openclaw.AgentProfile | 'local') => {
        setIsLoading(true);
        try {
            if (profile === 'local') {
                await openclaw.saveGatewaySettings('local', '', '');
            } else {
                await openclaw.switchToProfile(profile.id);
            }
            setIsAgentListOpen(false);
            // Wait a bit for restart
            setTimeout(fetchData, 1000);
        } catch (e) {
            console.error('Failed to switch agent:', e);
        }
    };

    const handleSetDefault = async (e: React.MouseEvent, agentId: string) => {
        e.stopPropagation();
        try {
            await openclaw.setDefaultAgent(agentId);
            toast.success('Default agent updated');
            fetchData();
        } catch (err) {
            toast.error(`Failed to set default: ${err}`);
        }
    };

    // Identify active agent
    const activeAgentName = status?.gateway_mode === 'local'
        ? 'Local Core'
        : (status?.profiles.find(p => p.url === status.remote_url)?.name || 'Remote Agent');

    const ActiveAgentIcon = status?.gateway_mode === 'local' ? Laptop : Server;

    return (
        <motion.div
            className="flex flex-col flex-1 h-full"
            variants={containerVariants}
            initial="hidden"
            animate="visible"
        >
            {/* Header */}
            <div className="flex items-center gap-3 px-1 mb-4">
                <button
                    onClick={onBack}
                    className="w-8 h-8 rounded-lg bg-muted/50 hover:bg-muted flex items-center justify-center shrink-0 transition-colors"
                >
                    <ChevronLeft className="w-4 h-4" />
                </button>
                <div className={cn("flex items-center gap-2", !sidebarOpen && "hidden")}>
                    <Radio className="w-4 h-4 text-primary" />
                    <span className="font-bold text-base">OpenClaw</span>
                </div>
            </div>

            {/* Agent Switcher */}
            <div className={cn("mb-6 px-3 relative", !sidebarOpen && "px-1")}>
                <button
                    onClick={() => sidebarOpen && setIsAgentListOpen(!isAgentListOpen)}
                    className={cn(
                        "w-full flex items-center gap-3 p-2 rounded-xl transition-all border",
                        isAgentListOpen ? "bg-accent border-primary/20" : "bg-card/50 border-white/5 hover:bg-accent/50",
                        !sidebarOpen && "justify-center px-0 border-none bg-transparent"
                    )}
                >
                    <div className={cn(
                        "w-8 h-8 rounded-lg flex items-center justify-center shrink-0 transition-colors",
                        status?.gateway_running ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground"
                    )}>
                        <ActiveAgentIcon className="w-4 h-4" />
                    </div>

                    {sidebarOpen && (
                        <>
                            <div className="flex-1 text-left min-w-0">
                                <p className="text-xs font-bold text-foreground truncate">{activeAgentName}</p>
                                <p className="text-[10px] text-muted-foreground flex items-center gap-1.5">
                                    <span className={cn("w-1.5 h-1.5 rounded-full", status?.gateway_running ? "bg-emerald-500 animate-pulse" : "bg-red-500")} />
                                    {status?.gateway_running ? "Online" : "Offline"}
                                </p>
                            </div>
                            <ChevronDown className={cn("w-3 h-3 text-muted-foreground transition-transform", isAgentListOpen && "rotate-180")} />
                        </>
                    )}
                </button>

                {/* Dropdown Menu */}
                {isAgentListOpen && sidebarOpen && (
                    <div className="absolute top-full left-3 right-3 mt-2 p-1.5 bg-zinc-900 border border-border rounded-xl shadow-2xl z-50 animate-in fade-in zoom-in-95 duration-200">
                        <div className="space-y-0.5">
                            <button
                                onClick={() => handleSwitchAgent('local')}
                                className={cn(
                                    "w-full flex items-center gap-3 p-2 rounded-lg text-left transition-colors",
                                    status?.gateway_mode === 'local' ? "bg-primary/10 text-primary" : "hover:bg-white/5 text-muted-foreground hover:text-foreground"
                                )}
                            >
                                <Laptop className="w-4 h-4" />
                                <div className="flex-1">
                                    <p className="text-xs font-bold">Local Core</p>
                                    <p className="text-[9px] opacity-70">Running on this device</p>
                                </div>
                                {status?.gateway_mode === 'local' && <div className="w-1.5 h-1.5 rounded-full bg-primary" />}
                            </button>

                            {(status?.profiles || []).map(profile => (
                                <button
                                    key={profile.id}
                                    onClick={() => handleSwitchAgent(profile)}
                                    className={cn(
                                        "w-full flex items-center gap-3 p-2 rounded-lg text-left transition-colors",
                                        status?.gateway_mode === 'remote' && status.remote_url === profile.url
                                            ? "bg-primary/10 text-primary"
                                            : "hover:bg-white/5 text-muted-foreground hover:text-foreground"
                                    )}
                                >
                                    <Server className="w-4 h-4" />
                                    <div className="flex-1 min-w-0">
                                        <div className="flex items-center gap-1.5">
                                            <p className="text-xs font-bold truncate">{profile.name}</p>
                                            {profile.is_default && <Star className="w-3 h-3 text-amber-400 fill-amber-400" />}
                                            {profile.status && (
                                                <span className={cn("w-1.5 h-1.5 rounded-full",
                                                    profile.status === 'running' ? 'bg-emerald-500' :
                                                        profile.status === 'paused' ? 'bg-amber-500' :
                                                            profile.status === 'error' ? 'bg-red-500' : 'bg-zinc-500'
                                                )} />
                                            )}
                                        </div>
                                        <p className="text-[9px] opacity-70 truncate">
                                            {profile.url}
                                            {profile.session_count != null && ` · ${profile.session_count} sessions`}
                                        </p>
                                    </div>
                                    <div className="flex items-center gap-1">
                                        {!profile.is_default && (
                                            <button
                                                onClick={(e) => handleSetDefault(e, profile.id)}
                                                className="p-1 rounded hover:bg-white/10 transition-colors"
                                                title="Set as default"
                                            >
                                                <Star className="w-3 h-3 text-muted-foreground/40 hover:text-amber-400" />
                                            </button>
                                        )}
                                        {status?.gateway_mode === 'remote' && status.remote_url === profile.url && <div className="w-1.5 h-1.5 rounded-full bg-primary" />}
                                    </div>
                                </button>
                            ))}
                        </div>

                        <div className="mt-2 pt-2 border-t border-white/5">
                            <button
                                onClick={() => {
                                    setIsAgentListOpen(false);
                                    onNavigateToSettings('openclaw-gateway');
                                }}
                                className="w-full flex items-center gap-2 p-2 rounded-lg text-xs font-medium text-muted-foreground hover:text-foreground hover:bg-white/5 transition-colors"
                            >
                                <Settings className="w-3 h-3" />
                                Manage Agents
                            </button>
                        </div>
                    </div>
                )}
            </div>

            {/* Navigation */}
            <div className="space-y-1 mb-6">
                {[
                    { id: 'dashboard', label: 'Dashboard', icon: Layout },
                    { id: 'chat', label: 'Live Chat', icon: MessageCircle },
                    { id: 'fleet', label: 'Fleet Command', icon: Server },
                    { id: 'brain', label: 'The Brain', icon: Brain },
                    { id: 'memory', label: 'Temporal Memory', icon: History },
                    { id: 'channels', label: 'Channels', icon: Smartphone },
                    { id: 'channel-status', label: 'Channel Status', icon: Zap },
                    { id: 'presence', label: 'Presence', icon: Cpu },
                    { id: 'automations', label: 'Automations', icon: Timer },
                    { id: 'routine-audit', label: 'Routine Audit', icon: FileText },
                    { id: 'skills', label: 'Skills', icon: Package },
                    { id: 'hooks', label: 'Hooks', icon: Anchor },
                    { id: 'plugins', label: 'Plugins', icon: Plug },
                    { id: 'config', label: 'Config Editor', icon: Settings2 },
                    { id: 'tool-policies', label: 'Tool Policies', icon: Wrench },
                    { id: 'pairing', label: 'DM Pairing', icon: KeyRound },
                    { id: 'cost-dashboard', label: 'Cost Dashboard', icon: DollarSign },
                    { id: 'routing', label: 'Routing', icon: GitBranch },
                    { id: 'cache-stats', label: 'Cache Stats', icon: Database },
                    { id: 'event-inspector', label: 'Event Inspector', icon: Activity },
                    { id: 'doctor', label: 'Doctor', icon: Stethoscope },
                    { id: 'system-control', label: 'System', icon: Shield },
                ].map((item) => {
                    const isDisabled = item.id === 'skills' && !gatewayRunning;
                    return (
                        <motion.button
                            key={item.id}
                            variants={itemVariants}
                            onClick={() => !isDisabled && onSelectPage(item.id as OpenClawPage)}
                            disabled={isDisabled}
                            className={cn(
                                "flex items-center gap-2 rounded-lg transition-all duration-300",
                                sidebarOpen ? "w-full px-3 py-2" : "w-10 h-10 justify-center mx-auto",
                                activePage === item.id
                                    ? "bg-accent text-foreground font-semibold shadow-sm ring-1 ring-primary/20"
                                    : "text-muted-foreground hover:bg-muted hover:text-foreground",
                                isDisabled && "opacity-40 cursor-not-allowed grayscale-[0.5] hover:bg-transparent"
                            )}
                            title={isDisabled ? `${item.label} (Requires active Gateway)` : (!sidebarOpen ? item.label : undefined)}
                        >
                            <item.icon className={cn("w-4 h-4 shrink-0 transition-colors duration-300", activePage === item.id && !isDisabled ? "text-primary" : "group-hover:text-primary")} />
                            <span className={cn("transition-all duration-300 text-sm", sidebarOpen ? "opacity-100" : "opacity-0 hidden")}>
                                {item.label}
                            </span>
                        </motion.button>
                    );
                })}
            </div>

            {/* Content List (Sessions) */}
            <div className="flex-1 overflow-y-auto space-y-1">
                {activePage === 'chat' && (
                    <>
                        {gatewayRunning && (
                            <button
                                onClick={onNewSession}
                                className={cn(
                                    "flex items-center gap-2 rounded-lg bg-primary/10 text-primary text-xs font-bold uppercase tracking-wider transition-all duration-300 mb-4 border border-primary/10 hover:bg-primary/20",
                                    sidebarOpen ? "w-full px-3 py-2.5 justify-start" : "w-10 h-10 justify-center mx-auto"
                                )}
                            >
                                <MessageCircle className="w-4 h-4" />
                                <span className={cn(sidebarOpen ? "block" : "hidden")}>New Session</span>
                            </button>
                        )}

                        {!gatewayRunning ? (
                            <div className={cn("text-center py-8 text-muted-foreground text-xs", !sidebarOpen && "hidden")}>
                                <p>Gateway not running</p>
                            </div>
                        ) : sessions.length === 0 ? (
                            <div className={cn("text-center py-8 text-muted-foreground text-xs", !sidebarOpen && "hidden")}>
                                <p>No sessions found</p>
                            </div>
                        ) : (
                            sessions.map((session) => (
                                <div key={session.session_key} className="relative group">
                                    <button
                                        onClick={() => onSelectSession(session.session_key)}
                                        className={cn(
                                            "w-full text-left rounded-lg transition-all",
                                            sidebarOpen ? "px-3 py-2 hover:bg-accent pr-8" : "w-10 h-10 flex items-center justify-center hover:bg-accent mx-auto",
                                            selectedSessionKey === session.session_key && "bg-accent border border-white/5"
                                        )}
                                    >
                                        {sidebarOpen ? (
                                            <div className="flex items-start gap-2">
                                                <MessageCircle className={cn("w-4 h-4 mt-0.5 shrink-0", session.session_key === 'agent:main' ? "text-blue-400" : "text-muted-foreground")} />
                                                <div className="flex-1 min-w-0">
                                                    <p className={cn("text-sm truncate", session.session_key === 'agent:main' ? "font-bold text-blue-100" : "font-medium")}>
                                                        {session.session_key === 'agent:main' ? 'OpenClaw Core' : (session.title || session.session_key.split(':').pop()?.slice(0, 8))}
                                                    </p>
                                                    <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
                                                        <span>{session.source || 'system'}</span>
                                                    </div>
                                                </div>
                                            </div>
                                        ) : (
                                            <MessageCircle className={cn("w-4 h-4", session.session_key === 'agent:main' ? "text-blue-400" : "text-muted-foreground")} />
                                        )}
                                    </button>
                                    {sidebarOpen && session.session_key !== 'agent:main' && (
                                        <button
                                            onClick={(e) => handleDeleteSession(e, session.session_key)}
                                            className={cn(
                                                "absolute right-2 top-1/2 -translate-y-1/2 p-1.5 rounded-md transition-all z-10",
                                                pendingDeleteKey === session.session_key
                                                    ? "opacity-100 bg-red-500/20 text-red-500 animate-pulse"
                                                    : "opacity-0 group-hover:opacity-100 hover:bg-red-500/10 text-muted-foreground hover:text-red-500"
                                            )}
                                            title={pendingDeleteKey === session.session_key ? "Click again to confirm delete" : "Delete Session"}
                                        >
                                            <Trash2 className="w-3.5 h-3.5" />
                                        </button>
                                    )}
                                </div>
                            ))
                        )}
                    </>
                )}
            </div>

            {/* Bottom Actions */}
            <div className="mt-auto pt-4 border-t border-border/50 space-y-1">
                <button
                    onClick={() => onNavigateToSettings('openclaw-gateway')}
                    className={cn(
                        "flex items-center gap-2 text-xs font-medium text-muted-foreground hover:text-foreground transition-all duration-300 rounded-lg hover:bg-accent",
                        sidebarOpen ? "w-full px-3 py-2" : "w-10 h-10 justify-center mx-auto"
                    )}
                >
                    <Settings className="w-4 h-4" />
                    {sidebarOpen && "Gateway Settings"}
                </button>
                {activePage === 'chat' && status?.gateway_running && (
                    <button
                        onClick={fetchData}
                        className={cn(
                            "flex items-center gap-2 text-xs font-medium text-muted-foreground hover:text-foreground transition-all duration-300 rounded-lg hover:bg-accent",
                            sidebarOpen ? "w-full px-3 py-2" : "w-10 h-10 justify-center mx-auto"
                        )}
                    >
                        <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                        {sidebarOpen && "Refresh Sessions"}
                    </button>
                )}
            </div>
        </motion.div>
    );
}
