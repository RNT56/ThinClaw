import { describe, expect, it } from 'vitest';

import {
    AGENT_PRIMARY_PAGE_IDS,
    AGENT_ROUTE_ALIASES,
    AGENT_ROUTE_REGISTRY,
    resolveAgentRoute,
} from '../../components/thinclaw/agent-routes';

const legacyPages = [
    'chat', 'dashboard', 'fleet', 'channels', 'channel-status', 'presence',
    'automations', 'jobs', 'repo-projects', 'autonomy', 'routine-audit', 'skills',
    'hooks', 'plugins', 'system-control', 'brain', 'memory', 'config',
    'event-inspector', 'doctor', 'tool-policies', 'pairing', 'cost-dashboard',
    'cache-stats', 'routing', 'experiments', 'learning', 'trajectory', 'rollback',
    'session-search', 'channel-config', 'remote-access',
] as const;

describe('Agent Cockpit route registry', () => {
    it('keeps exactly ten primary sidebar destinations', () => {
        expect(AGENT_PRIMARY_PAGE_IDS).toHaveLength(10);
        expect(AGENT_ROUTE_REGISTRY.map((route) => route.id)).toEqual(AGENT_PRIMARY_PAGE_IDS);
    });

    it('resolves every legacy sidebar destination to a valid primary destination', () => {
        expect(legacyPages).toHaveLength(32);
        for (const page of legacyPages) {
            const resolved = resolveAgentRoute(page);
            expect(AGENT_PRIMARY_PAGE_IDS).toContain(resolved.destination);
            expect(AGENT_ROUTE_ALIASES[page]).toEqual(resolved);
        }
    });

    it('quarantines repo projects and keeps session search inside Chat', () => {
        expect(resolveAgentRoute('repo-projects')).toEqual({ destination: 'advanced', tab: 'projects' });
        expect(resolveAgentRoute('session-search')).toEqual({ destination: 'chat', tab: 'search' });
    });
});
