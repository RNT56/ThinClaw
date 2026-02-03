import { useState, useEffect, useCallback } from 'react';
import {
    MessageCircle, Radio, ChevronLeft, RefreshCw, Settings,
    Layout, Smartphone, Timer, Package, Cpu, Shield, Brain, History
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as clawdbot from '../../lib/clawdbot';
import { ClawdbotSession } from '../../lib/clawdbot';

import { motion } from 'framer-motion';

export type ClawdbotPage = 'chat' | 'dashboard' | 'channels' | 'presence' | 'automations' | 'skills' | 'system-control' | 'brain' | 'memory';

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

interface ClawdbotSidebarProps {
    sidebarOpen: boolean;
    onBack: () => void;
    onSelectSession: (sessionKey: string) => void;
    onNewSession: () => void;
    selectedSessionKey: string | null;
    gatewayRunning: boolean;
    onNavigateToSettings: (page: 'clawdbot-gateway') => void;
    activePage: ClawdbotPage;
    onSelectPage: (page: ClawdbotPage) => void;
}

export function ClawdbotSidebar({
    sidebarOpen,
    onBack,
    onSelectSession,
    onNewSession,
    selectedSessionKey,
    gatewayRunning,
    onNavigateToSettings,
    activePage,
    onSelectPage
}: ClawdbotSidebarProps) {
    const [sessions, setSessions] = useState<ClawdbotSession[]>([]);
    const [isLoading, setIsLoading] = useState(false);

    const fetchSessions = useCallback(async () => {
        if (!gatewayRunning) return;
        setIsLoading(true);
        try {
            const res = await clawdbot.getClawdbotSessions();
            setSessions(res.sessions);
        } catch (e) {
            console.error('Failed to fetch sessions:', e);
        } finally {
            setIsLoading(false);
        }
    }, [gatewayRunning]);

    useEffect(() => {
        if (activePage === 'chat') {
            fetchSessions();
        }
        const interval = setInterval(() => {
            if (activePage === 'chat') fetchSessions();
        }, 10000);
        return () => clearInterval(interval);
    }, [fetchSessions, activePage]);

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
                    <span className="font-bold text-base">Clawdbot</span>
                </div>
            </div>

            {/* Gateway Status Badge */}
            <div className={cn("mb-4 px-1", !sidebarOpen && "flex justify-center")}>
                <div className={cn(
                    "flex items-center gap-2 px-2 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-wider",
                    gatewayRunning ? "bg-green-500/10 text-green-500 border border-green-500/10" : "bg-muted text-muted-foreground"
                )}>
                    <div className={cn("w-1.5 h-1.5 rounded-full", gatewayRunning ? "bg-green-500 animate-pulse" : "bg-muted-foreground")} />
                    {sidebarOpen && (gatewayRunning ? "Gateway Active" : "Gateway Offline")}
                </div>
            </div>

            {/* Navigation */}
            <div className="space-y-1 mb-6">
                {[
                    { id: 'dashboard', label: 'Dashboard', icon: Layout },
                    { id: 'chat', label: 'Live Chat', icon: MessageCircle },
                    { id: 'brain', label: 'The Brain', icon: Brain },
                    { id: 'memory', label: 'Temporal Memory', icon: History },
                    { id: 'channels', label: 'Channels', icon: Smartphone },
                    { id: 'presence', label: 'Presence', icon: Cpu },
                    { id: 'automations', label: 'Automations', icon: Timer },
                    { id: 'skills', label: 'Skills', icon: Package },
                    { id: 'system-control', label: 'System', icon: Shield },
                ].map((item) => {
                    const isDisabled = item.id === 'skills' && !gatewayRunning;
                    return (
                        <motion.button
                            key={item.id}
                            variants={itemVariants}
                            onClick={() => !isDisabled && onSelectPage(item.id as ClawdbotPage)}
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
                                </div>
                            ))
                        )}
                    </>
                )}
            </div>

            {/* Bottom Actions */}
            <div className="mt-auto pt-4 border-t border-border/50 space-y-1">
                <button
                    onClick={() => onNavigateToSettings('clawdbot-gateway')}
                    className={cn(
                        "flex items-center gap-2 text-xs font-medium text-muted-foreground hover:text-foreground transition-all duration-300 rounded-lg hover:bg-accent",
                        sidebarOpen ? "w-full px-3 py-2" : "w-10 h-10 justify-center mx-auto"
                    )}
                >
                    <Settings className="w-4 h-4" />
                    {sidebarOpen && "Gateway Settings"}
                </button>
                {activePage === 'chat' && gatewayRunning && (
                    <button
                        onClick={fetchSessions}
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
