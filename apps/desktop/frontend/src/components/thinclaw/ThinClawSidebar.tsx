import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
    ChevronDown,
    ChevronLeft,
    CircleAlert,
    Laptop,
    MessageCircle,
    Plus,
    Radio,
    RefreshCw,
    Search,
    Server,
    Settings,
    Trash2,
} from 'lucide-react';
import { motion } from 'framer-motion';
import { toast } from 'sonner';

import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { ConfirmDialog, Notice, StatusBadge } from '../ui';
import { useAgentCockpit } from './AgentCockpitProvider';
import {
    AGENT_ROUTE_REGISTRY,
    AGENT_ROUTE_SECTIONS,
    type ThinClawPage,
    resolveAgentRoute,
} from './agent-routes';

export type { ThinClawPage } from './agent-routes';

interface ThinClawSidebarProps {
    sidebarOpen: boolean;
    onBack: () => void;
    onSelectSession: (sessionKey: string) => void;
    onNewSession: () => void;
    selectedSessionKey: string | null;
    gatewayRunning: boolean;
    onNavigateToSettings: (page: 'thinclaw-gateway') => void;
    activePage: ThinClawPage;
    onSelectPage: (page: ThinClawPage) => void;
}

function profileName(status: thinclaw.ThinClawStatus | null) {
    if (!status) return 'Checking profile';
    if (status.gateway_mode === 'local') return 'Local Core';
    return status.profiles.find((profile) => profile.url === status.remote_url)?.name ?? 'Remote profile';
}

function profileConnection(status: thinclaw.ThinClawStatus | null) {
    if (!status) return 'Checking';
    if (status.engine_running && status.engine_connected) return 'Connected';
    if (status.gateway_mode === 'remote') return 'Disconnected';
    return 'Stopped';
}

export function ThinClawSidebar({
    sidebarOpen,
    onBack,
    onSelectSession,
    onNewSession,
    selectedSessionKey,
    gatewayRunning,
    onNavigateToSettings,
    activePage,
    onSelectPage,
}: ThinClawSidebarProps) {
    const { status, checkedAt, error, isRefreshing, refresh, capability } = useAgentCockpit();
    const [sessions, setSessions] = useState<thinclaw.ThinClawSession[]>([]);
    const [isLoadingSessions, setIsLoadingSessions] = useState(false);
    const [agentMenuOpen, setAgentMenuOpen] = useState(false);
    const [switchError, setSwitchError] = useState<string | null>(null);
    const [isSwitching, setIsSwitching] = useState(false);
    const [deleteTarget, setDeleteTarget] = useState<thinclaw.ThinClawSession | null>(null);
    const [isDeletingSession, setIsDeletingSession] = useState(false);
    const navRefs = useRef<Record<string, HTMLButtonElement | null>>({});

    const selectedDestination = resolveAgentRoute(activePage).destination;
    const runtimeCapability = capability('runtime');
    const profileCapability = capability('always');
    const activeProfileName = profileName(status);
    const connection = profileConnection(status);
    const profileIcon = status?.gateway_mode === 'remote' ? Server : Laptop;
    const ProfileIcon = profileIcon;
    const freshness = checkedAt ? new Date(checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;

    const loadSessions = useCallback(async () => {
        if (!status?.engine_running || selectedDestination !== 'chat') {
            if (!status?.engine_running) setSessions([]);
            return;
        }
        setIsLoadingSessions(true);
        try {
            const response = await thinclaw.getThinClawSessions();
            setSessions(response.sessions ?? []);
        } catch (caught) {
            setSessions([]);
            toast.error(`Unable to load agent sessions: ${caught instanceof Error ? caught.message : String(caught)}`);
        } finally {
            setIsLoadingSessions(false);
        }
    }, [selectedDestination, status?.engine_running]);

    useEffect(() => {
        void loadSessions();
    }, [loadSessions]);

    useEffect(() => {
        if (selectedDestination !== 'chat' || !status?.engine_running) return;
        const poll = window.setInterval(() => void loadSessions(), 15_000);
        return () => window.clearInterval(poll);
    }, [loadSessions, selectedDestination, status?.engine_running]);

    const switchProfile = async (profile: thinclaw.AgentProfile | 'local') => {
        setIsSwitching(true);
        setSwitchError(null);
        try {
            if (profile === 'local') await thinclaw.saveGatewaySettings('local', '', '');
            else await thinclaw.switchToProfile(profile.id);
            setAgentMenuOpen(false);
            await refresh();
        } catch (caught) {
            setSwitchError(caught instanceof Error ? caught.message : String(caught));
        } finally {
            setIsSwitching(false);
        }
    };

    const deleteSession = async () => {
        if (!deleteTarget) return;
        setIsDeletingSession(true);
        try {
            await thinclaw.deleteThinClawSession(deleteTarget.session_key);
            setSessions((current) => current.filter((session) => session.session_key !== deleteTarget.session_key));
            if (selectedSessionKey === deleteTarget.session_key) onSelectSession('agent:main');
            setDeleteTarget(null);
            toast.success('Session deleted');
        } catch (caught) {
            toast.error(`Session could not be deleted. Stop its active run first if needed: ${caught instanceof Error ? caught.message : String(caught)}`);
        } finally {
            setIsDeletingSession(false);
        }
    };

    const routesBySection = useMemo(() => AGENT_ROUTE_SECTIONS.map((section) => ({
        ...section,
        routes: AGENT_ROUTE_REGISTRY.filter((route) => route.section === section.id),
    })), []);

    const navigableRoutes = AGENT_ROUTE_REGISTRY.filter((route) => {
        const state = capability(route.capability).state;
        return route.capability === 'always' || route.capability === 'advanced' || state === 'available' || state === 'stale';
    });
    const rovingRouteId = navigableRoutes.some((route) => route.id === selectedDestination)
        ? selectedDestination
        : navigableRoutes[0]?.id;

    const moveNavFocus = (routeId: string, destination: 'first' | 'last' | number) => {
        if (navigableRoutes.length === 0) return;
        const index = Math.max(0, navigableRoutes.findIndex((route) => route.id === routeId));
        const next = destination === 'first'
            ? navigableRoutes[0]
            : destination === 'last'
                ? navigableRoutes[navigableRoutes.length - 1]
                : navigableRoutes[(index + destination + navigableRoutes.length) % navigableRoutes.length];
        if (!next) return;
        onSelectPage(next.id);
        requestAnimationFrame(() => navRefs.current[next.id]?.focus());
    };

    return (
        <motion.nav
            aria-label="Agent Cockpit navigation"
            className="flex h-full min-h-0 flex-1 flex-col"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            <div className="mb-4 flex shrink-0 items-center gap-3 px-1">
                <button type="button" onClick={onBack} aria-label="Back to Direct Workbench" className="grid size-8 place-items-center rounded-[var(--radius-control)] bg-surface-subtle text-content-muted transition-colors hover:bg-surface-panel hover:text-content-primary">
                    <ChevronLeft className="size-4" aria-hidden="true" />
                </button>
                {sidebarOpen && <div className="flex items-center gap-2"><Radio className="size-4 text-primary" aria-hidden="true" /><span className="text-base font-semibold">ThinClaw</span></div>}
            </div>

            <div className={cn('relative mb-3 shrink-0', sidebarOpen ? 'px-2' : 'px-0')}>
                <button
                    type="button"
                    onClick={() => sidebarOpen && setAgentMenuOpen((open) => !open)}
                    aria-expanded={agentMenuOpen}
                    className={cn(
                        'flex w-full items-center gap-3 rounded-[var(--radius-panel)] border border-surface-outline bg-surface-panel p-2 text-left transition-colors hover:bg-surface-subtle',
                        !sidebarOpen && 'justify-center border-transparent bg-transparent px-0',
                    )}
                >
                    <span className="grid size-8 shrink-0 place-items-center rounded-[var(--radius-control)] bg-primary/10 text-primary"><ProfileIcon className="size-4" aria-hidden="true" /></span>
                    {sidebarOpen && <span className="min-w-0 flex-1"><span className="block truncate text-xs font-semibold text-content-primary">{activeProfileName}</span><span className="mt-0.5 flex items-center gap-1.5 text-[10px] text-content-muted"><StatusBadge status={connection} className="px-1.5 py-0 text-[9px]" />{freshness ? `Checked ${freshness}` : 'Checking status'}</span></span>}
                    {sidebarOpen && <ChevronDown className={cn('size-3 shrink-0 text-content-muted transition-transform', agentMenuOpen && 'rotate-180')} aria-hidden="true" />}
                </button>

                {agentMenuOpen && sidebarOpen && (
                    <div className="absolute inset-x-2 top-full z-50 mt-2 rounded-[var(--radius-panel)] border border-surface-outline bg-surface-elevated p-2 shadow-lg">
                        <button type="button" onClick={() => void switchProfile('local')} disabled={isSwitching} className={cn('flex w-full items-center gap-3 rounded-[var(--radius-control)] p-2 text-left text-xs transition-colors hover:bg-surface-subtle', status?.gateway_mode === 'local' && 'bg-primary/10 text-primary')}>
                            <Laptop className="size-4" aria-hidden="true" /><span className="min-w-0 flex-1"><span className="block font-semibold">Local Core</span><span className="block text-[10px] text-content-muted">Runs on this Desktop</span></span>
                        </button>
                        {(status?.profiles ?? []).map((profile) => (
                            <button key={profile.id} type="button" onClick={() => void switchProfile(profile)} disabled={isSwitching} className={cn('mt-1 flex w-full items-center gap-3 rounded-[var(--radius-control)] p-2 text-left text-xs transition-colors hover:bg-surface-subtle', status?.gateway_mode === 'remote' && status.remote_url === profile.url && 'bg-primary/10 text-primary')}>
                                <Server className="size-4 shrink-0" aria-hidden="true" /><span className="min-w-0 flex-1"><span className="block truncate font-semibold">{profile.name}</span><span className="block truncate text-[10px] text-content-muted">{profile.url}</span></span>
                            </button>
                        ))}
                        <button type="button" onClick={() => { setAgentMenuOpen(false); onNavigateToSettings('thinclaw-gateway'); }} className="mt-2 flex w-full items-center gap-2 border-t border-surface-outline px-2 pt-3 text-xs text-content-muted hover:text-content-primary">
                            <Settings className="size-3.5" aria-hidden="true" /> Manage agents
                        </button>
                    </div>
                )}
            </div>

            {switchError && sidebarOpen && <Notice tone="error" title="Profile switch failed" className="mx-2 mb-3">{switchError}</Notice>}
            {error && sidebarOpen && <Notice tone="warning" title="Profile status needs attention" className="mx-2 mb-3">{error}</Notice>}

            <div className="min-h-0 flex-1 overflow-y-auto">
                <div className="space-y-3 pb-3">
                    {routesBySection.map((section) => (
                        <section key={section.id} aria-label={section.label}>
                            {sidebarOpen && <p className="mb-1 px-3 text-[10px] font-bold uppercase tracking-widest text-content-muted">{section.label}</p>}
                            <div className="space-y-0.5">
                                {section.routes.map((route) => {
                                    const state = capability(route.capability);
                                    const canRemediate = route.capability === 'always' || route.capability === 'advanced';
                                    const disabled = !canRemediate && (state.state === 'unavailable' || state.state === 'loading');
                                    const selected = selectedDestination === route.id;
                                    const Icon = route.icon;
                                    return (
                                        <button
                                            key={route.id}
                                            ref={(node) => { navRefs.current[route.id] = node; }}
                                            type="button"
                                            aria-current={selected ? 'page' : undefined}
                                            tabIndex={disabled ? -1 : route.id === rovingRouteId ? 0 : -1}
                                            disabled={disabled}
                                            title={disabled ? [state.reason, state.remediation].filter(Boolean).join(' ') : (!sidebarOpen ? route.label : undefined)}
                                            onClick={() => onSelectPage(route.id)}
                                            onKeyDown={(event) => {
                                                if (event.key === 'ArrowDown') { event.preventDefault(); moveNavFocus(route.id, 1); }
                                                if (event.key === 'ArrowUp') { event.preventDefault(); moveNavFocus(route.id, -1); }
                                                if (event.key === 'Home') { event.preventDefault(); moveNavFocus(route.id, 'first'); }
                                                if (event.key === 'End') { event.preventDefault(); moveNavFocus(route.id, 'last'); }
                                            }}
                                            className={cn(
                                                'flex items-center gap-2 rounded-[var(--radius-control)] text-left text-xs transition-colors',
                                                sidebarOpen ? 'w-full px-3 py-2' : 'mx-auto size-9 justify-center',
                                                selected ? 'bg-primary/10 font-semibold text-content-primary ring-1 ring-primary/20' : 'text-content-muted hover:bg-surface-subtle hover:text-content-primary',
                                                disabled && 'cursor-not-allowed opacity-45 hover:bg-transparent',
                                            )}
                                        >
                                            <Icon className={cn('size-3.5 shrink-0', selected && 'text-primary')} aria-hidden="true" />
                                            {sidebarOpen && <span className="truncate">{route.label}</span>}
                                        </button>
                                    );
                                })}
                            </div>
                        </section>
                    ))}
                </div>

                {selectedDestination === 'chat' && (
                    <section className="border-t border-surface-outline pt-3" aria-label="Chat sessions">
                        {sidebarOpen && <p className="mb-1 px-3 text-[10px] font-bold uppercase tracking-widest text-content-muted">Sessions</p>}
                        <div className="space-y-0.5">
                            {runtimeCapability.state === 'available' && (
                                <button type="button" onClick={onNewSession} className={cn('flex items-center gap-2 rounded-[var(--radius-control)] bg-primary/10 text-xs font-semibold text-primary transition-colors hover:bg-primary/15', sidebarOpen ? 'w-full px-3 py-2' : 'mx-auto size-9 justify-center')} title={!sidebarOpen ? 'New session' : undefined}>
                                    <Plus className="size-3.5" aria-hidden="true" />{sidebarOpen && 'New session'}
                                </button>
                            )}
                            <button type="button" onClick={() => onSelectPage('session-search')} className={cn('flex items-center gap-2 rounded-[var(--radius-control)] text-xs text-content-muted transition-colors hover:bg-surface-subtle hover:text-content-primary', sidebarOpen ? 'w-full px-3 py-2' : 'mx-auto size-9 justify-center')} title={!sidebarOpen ? 'Search sessions' : undefined}>
                                <Search className="size-3.5" aria-hidden="true" />{sidebarOpen && 'Search sessions'}
                            </button>
                            {runtimeCapability.state !== 'available' ? (
                                sidebarOpen && <p className="px-3 py-3 text-xs text-content-muted">{runtimeCapability.reason}</p>
                            ) : sessions.length === 0 ? (
                                sidebarOpen && <p className="px-3 py-3 text-xs text-content-muted">{isLoadingSessions ? 'Loading sessions…' : 'No sessions found.'}</p>
                            ) : sessions.map((session) => (
                                <div key={session.session_key} className="group relative">
                                    <button type="button" onClick={() => onSelectSession(session.session_key)} className={cn('flex w-full items-center gap-2 rounded-[var(--radius-control)] text-left transition-colors hover:bg-surface-subtle', sidebarOpen ? 'px-3 py-2 pr-8' : 'mx-auto size-9 justify-center', selectedSessionKey === session.session_key && 'bg-surface-subtle text-content-primary')} title={!sidebarOpen ? (session.title ?? session.session_key) : undefined}>
                                        <MessageCircle className="size-3.5 shrink-0 text-content-muted" aria-hidden="true" />
                                        {sidebarOpen && <span className="min-w-0 flex-1"><span className="block truncate text-xs font-medium">{session.session_key === 'agent:main' ? 'ThinClaw Core' : session.title ?? session.session_key}</span><span className="block truncate text-[10px] text-content-muted">{session.source ?? 'agent'}</span></span>}
                                    </button>
                                    {sidebarOpen && session.session_key !== 'agent:main' && (
                                        <button type="button" onClick={(event) => { event.preventDefault(); event.stopPropagation(); setDeleteTarget(session); }} className="absolute right-2 top-1/2 -translate-y-1/2 rounded p-1 text-content-muted opacity-0 transition-colors hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100 focus-visible:opacity-100" aria-label={`Delete ${session.title ?? session.session_key}`} title="Delete session">
                                            <Trash2 className="size-3" aria-hidden="true" />
                                        </button>
                                    )}
                                </div>
                            ))}
                        </div>
                    </section>
                )}
            </div>

            <div className="shrink-0 border-t border-surface-outline pt-2">
                <button type="button" onClick={() => onNavigateToSettings('thinclaw-gateway')} className={cn('flex items-center gap-2 rounded-[var(--radius-control)] text-xs text-content-muted transition-colors hover:bg-surface-subtle hover:text-content-primary', sidebarOpen ? 'w-full px-3 py-2' : 'mx-auto size-9 justify-center')} title={!sidebarOpen ? 'Gateway settings' : undefined}>
                    <Settings className="size-4" aria-hidden="true" />{sidebarOpen && 'Gateway settings'}
                </button>
                {(error || !checkedAt || profileCapability.state === 'stale') && (
                    <button type="button" onClick={() => void refresh()} disabled={isRefreshing} className={cn('mt-0.5 flex items-center gap-2 rounded-[var(--radius-control)] text-xs text-content-muted transition-colors hover:bg-surface-subtle hover:text-content-primary', sidebarOpen ? 'w-full px-3 py-2' : 'mx-auto size-9 justify-center')} title={!sidebarOpen ? 'Refresh profile' : undefined}>
                        <RefreshCw className={cn('size-4', isRefreshing && 'animate-spin')} aria-hidden="true" />{sidebarOpen && 'Refresh profile'}
                    </button>
                )}
                {!gatewayRunning && sidebarOpen && <div className="mt-2 flex items-center gap-2 px-3 text-[10px] text-content-muted"><CircleAlert className="size-3.5" aria-hidden="true" /> Gateway stopped</div>}
            </div>
            <ConfirmDialog
                open={deleteTarget !== null}
                onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
                title="Delete this agent session?"
                description={deleteTarget
                    ? <>Session <span className="font-mono">{deleteTarget.title ?? deleteTarget.session_key}</span> will be permanently removed. Stop any active run first.</>
                    : ''}
                confirmLabel="Delete session"
                onConfirm={deleteSession}
                isConfirming={isDeletingSession}
            />
        </motion.nav>
    );
}
