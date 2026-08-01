import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useState } from 'react';

import * as thinclaw from '../../lib/thinclaw';
import type { AgentCapabilityKey } from './agent-routes';

export type AgentAvailability = 'loading' | 'available' | 'unavailable' | 'degraded' | 'stale';

export interface AgentCapabilityState {
    state: AgentAvailability;
    reason?: string;
    remediation?: string;
    source: 'local' | 'remote' | 'unknown';
    checkedAt: string | null;
}

export interface AgentCockpitContextValue {
    status: thinclaw.ThinClawStatus | null;
    error: string | null;
    checkedAt: string | null;
    isRefreshing: boolean;
    source: 'local' | 'remote' | 'unknown';
    refresh: () => Promise<thinclaw.ThinClawStatus | null>;
    capability: (key: AgentCapabilityKey) => AgentCapabilityState;
}

const AgentCockpitContext = createContext<AgentCockpitContextValue | null>(null);

const FRESHNESS_MS = 30_000;

function errorMessage(error: unknown): string {
    if (error && typeof error === 'object') {
        const bridge = error as { reason?: string; message?: string; remediation?: string };
        const message = bridge.reason ?? bridge.message;
        if (message) return bridge.remediation ? `${message} — ${bridge.remediation}` : message;
    }
    return error instanceof Error ? error.message : String(error);
}

function sourceFor(status: thinclaw.ThinClawStatus | null): AgentCapabilityState['source'] {
    if (!status) return 'unknown';
    return status.gateway_mode.toLowerCase() === 'remote' ? 'remote' : 'local';
}

export function AgentCockpitProvider({ children, enabled = true }: { children: ReactNode; enabled?: boolean }) {
    const [status, setStatus] = useState<thinclaw.ThinClawStatus | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [checkedAt, setCheckedAt] = useState<string | null>(null);
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [now, setNow] = useState(() => Date.now());

    const refresh = useCallback(async () => {
        setIsRefreshing(true);
        try {
            const next = await thinclaw.getThinClawStatus();
            setStatus(next);
            setError(null);
            setCheckedAt(new Date().toISOString());
            return next;
        } catch (caught) {
            setError(errorMessage(caught));
            return null;
        } finally {
            setIsRefreshing(false);
        }
    }, []);

    useEffect(() => {
        if (!enabled) return;
        setNow(Date.now());
        void refresh();
        const poll = window.setInterval(() => void refresh(), 15_000);
        const freshnessClock = window.setInterval(() => setNow(Date.now()), 5_000);
        return () => {
            window.clearInterval(poll);
            window.clearInterval(freshnessClock);
        };
    }, [enabled, refresh]);

    const value = useMemo<AgentCockpitContextValue>(() => {
        const source = sourceFor(status);
        const checkedAtMs = checkedAt ? Date.parse(checkedAt) : 0;
        const stale = Boolean(status && checkedAtMs && now - checkedAtMs > FRESHNESS_MS);

        const base = (): AgentCapabilityState => {
            if (!status) {
                return {
                    state: error ? 'unavailable' : 'loading',
                    reason: error ?? 'Checking the selected agent profile.',
                    remediation: error ? 'Check the profile connection and try again.' : undefined,
                    source,
                    checkedAt,
                };
            }
            if (stale || error) {
                return {
                    state: 'stale',
                    reason: error ?? 'The last profile status is older than 30 seconds.',
                    remediation: 'Refresh the profile status before making a change.',
                    source,
                    checkedAt,
                };
            }
            return { state: 'available', source, checkedAt };
        };

        const capability = (key: AgentCapabilityKey): AgentCapabilityState => {
            const current = base();
            if (current.state !== 'available') return current;

            if (key === 'runtime') {
                if (status!.engine_running && status!.engine_connected) return current;
                return {
                    state: 'unavailable',
                    reason: source === 'remote'
                        ? 'The selected remote gateway is not connected.'
                        : 'The local gateway is stopped.',
                    remediation: source === 'remote'
                        ? 'Reconnect the selected profile in Gateway Settings.'
                        : 'Start the gateway in Operations & Safety.',
                    source,
                    checkedAt,
                };
            }
            if (key === 'local-host') {
                if (source === 'local') return current;
                return {
                    state: 'unavailable',
                    reason: 'Local host files belong to this Desktop, not the selected remote profile.',
                    remediation: 'Switch to Local Core to browse local host files.',
                    source,
                    checkedAt,
                };
            }
            if (key === 'local-subagent') {
                if (source === 'local') return current;
                return {
                    state: 'unavailable',
                    reason: 'Desktop-managed sub-agents run in Local Core, not the selected remote profile.',
                    remediation: 'Switch to Local Core to spawn or manage Desktop-managed sub-agents.',
                    source,
                    checkedAt,
                };
            }
            if (key === 'remote-access') {
                if (source === 'local') return current;
                return {
                    state: 'unavailable',
                    reason: 'Remote Access can only expose this Desktop’s local gateway.',
                    remediation: 'Switch to Local Core to manage Tailscale access.',
                    source,
                    checkedAt,
                };
            }
            return current;
        };

        return { status, error, checkedAt, isRefreshing, source, refresh, capability };
    }, [checkedAt, error, isRefreshing, now, refresh, status]);

    return <AgentCockpitContext.Provider value={value}>{children}</AgentCockpitContext.Provider>;
}

export function useAgentCockpit(): AgentCockpitContextValue {
    const value = useOptionalAgentCockpit();
    if (!value) throw new Error('useAgentCockpit must be used within AgentCockpitProvider');
    return value;
}

/** Optional form for legacy surfaces that still render outside the Cockpit shell. */
export function useOptionalAgentCockpit(): AgentCockpitContextValue | null {
    return useContext(AgentCockpitContext);
}

export function useAgentCapability(key: AgentCapabilityKey): AgentCapabilityState {
    return useAgentCockpit().capability(key);
}
