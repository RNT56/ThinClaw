import type { LucideIcon } from 'lucide-react';
import {
    Brain,
    Cable,
    CircleDollarSign,
    FlaskConical,
    LayoutDashboard,
    MessageCircle,
    Radio,
    ShieldCheck,
    Timer,
    Workflow,
} from 'lucide-react';

/**
 * The only destinations shown in the Agent Cockpit sidebar.  Legacy route IDs
 * are deliberately kept below as aliases while in-product links migrate.
 */
export const AGENT_PRIMARY_PAGE_IDS = [
    'home',
    'chat',
    'workspace',
    'channels',
    'automations',
    'jobs',
    'capabilities',
    'usage',
    'operations',
    'advanced',
] as const;

export type AgentPrimaryPage = typeof AGENT_PRIMARY_PAGE_IDS[number];

/** IDs that existed in the old 32-item sidebar and remain valid deep links. */
export type LegacyThinClawPage =
    | 'dashboard'
    | 'fleet'
    | 'channel-status'
    | 'presence'
    | 'repo-projects'
    | 'autonomy'
    | 'routine-audit'
    | 'skills'
    | 'hooks'
    | 'plugins'
    | 'system-control'
    | 'brain'
    | 'memory'
    | 'config'
    | 'event-inspector'
    | 'doctor'
    | 'tool-policies'
    | 'pairing'
    | 'cost-dashboard'
    | 'cache-stats'
    | 'routing'
    | 'experiments'
    | 'learning'
    | 'trajectory'
    | 'rollback'
    | 'session-search'
    | 'channel-config'
    | 'remote-access';

export type ThinClawPage = AgentPrimaryPage | LegacyThinClawPage;

export type AgentRouteSection = 'work' | 'manage' | 'operate';
export type AgentCapabilityKey = 'always' | 'runtime' | 'local-host' | 'local-subagent' | 'remote-access' | 'advanced';

export interface AgentRouteDefinition {
    id: AgentPrimaryPage;
    label: string;
    description: string;
    section: AgentRouteSection;
    icon: LucideIcon;
    capability: AgentCapabilityKey;
}

export const AGENT_ROUTE_REGISTRY: readonly AgentRouteDefinition[] = [
    {
        id: 'home',
        label: 'Home',
        description: 'Runtime health, blockers, and active work',
        section: 'work',
        icon: LayoutDashboard,
        capability: 'always',
    },
    {
        id: 'chat',
        label: 'Chat',
        description: 'Sessions, approvals, tools, and agent work',
        section: 'work',
        icon: MessageCircle,
        capability: 'runtime',
    },
    {
        id: 'workspace',
        label: 'Workspace & Memory',
        description: 'Agent documents, memory, and search',
        section: 'work',
        icon: Brain,
        capability: 'always',
    },
    {
        id: 'channels',
        label: 'Channels',
        description: 'Health, setup, pairing, and activity',
        section: 'manage',
        icon: Radio,
        capability: 'runtime',
    },
    {
        id: 'automations',
        label: 'Automations',
        description: 'Schedules, triggers, runs, and history',
        section: 'manage',
        icon: Timer,
        capability: 'runtime',
    },
    {
        id: 'jobs',
        label: 'Jobs',
        description: 'Running and historical agent execution',
        section: 'manage',
        icon: Workflow,
        capability: 'runtime',
    },
    {
        id: 'capabilities',
        label: 'Capabilities',
        description: 'Skills, extensions, tool access, and hooks',
        section: 'manage',
        icon: Cable,
        capability: 'runtime',
    },
    {
        id: 'usage',
        label: 'Usage',
        description: 'Cost, limits, and cache evidence',
        section: 'manage',
        icon: CircleDollarSign,
        capability: 'runtime',
    },
    {
        id: 'operations',
        label: 'Operations & Safety',
        description: 'Gateway, diagnostics, access, and checkpoints',
        section: 'operate',
        icon: ShieldCheck,
        capability: 'always',
    },
    {
        id: 'advanced',
        label: 'Advanced / Labs',
        description: 'Specialist and experimental controls',
        section: 'operate',
        icon: FlaskConical,
        capability: 'advanced',
    },
] as const;

export const AGENT_ROUTE_SECTIONS: readonly { id: AgentRouteSection; label: string }[] = [
    { id: 'work', label: 'Workspace' },
    { id: 'manage', label: 'Manage' },
    { id: 'operate', label: 'Operate' },
] as const;

export interface ResolvedAgentRoute {
    destination: AgentPrimaryPage;
    /** Optional tab ID consumed by the destination wrapper. */
    tab?: string;
}

/**
 * One alias map for the sidebar, renderer, command palette, and in-product
 * links. A legacy name never creates a second primary destination.
 */
export const AGENT_ROUTE_ALIASES: Readonly<Record<ThinClawPage, ResolvedAgentRoute>> = {
    home: { destination: 'home' },
    chat: { destination: 'chat' },
    workspace: { destination: 'workspace' },
    channels: { destination: 'channels', tab: 'overview' },
    automations: { destination: 'automations', tab: 'automations' },
    jobs: { destination: 'jobs' },
    capabilities: { destination: 'capabilities', tab: 'skills' },
    usage: { destination: 'usage', tab: 'cost' },
    operations: { destination: 'operations', tab: 'gateway' },
    advanced: { destination: 'advanced' },

    dashboard: { destination: 'home' },
    fleet: { destination: 'advanced', tab: 'fleet' },
    'channel-status': { destination: 'channels', tab: 'health' },
    presence: { destination: 'home' },
    'repo-projects': { destination: 'advanced', tab: 'projects' },
    autonomy: { destination: 'advanced', tab: 'autonomy' },
    'routine-audit': { destination: 'automations', tab: 'history' },
    skills: { destination: 'capabilities', tab: 'skills' },
    hooks: { destination: 'capabilities', tab: 'hooks' },
    plugins: { destination: 'capabilities', tab: 'extensions' },
    'system-control': { destination: 'operations', tab: 'gateway' },
    brain: { destination: 'workspace', tab: 'workspace' },
    memory: { destination: 'workspace', tab: 'memory' },
    config: { destination: 'advanced', tab: 'developer' },
    'event-inspector': { destination: 'advanced', tab: 'events' },
    doctor: { destination: 'operations', tab: 'diagnostics' },
    'tool-policies': { destination: 'capabilities', tab: 'tools' },
    pairing: { destination: 'channels', tab: 'security' },
    'cost-dashboard': { destination: 'usage', tab: 'cost' },
    'cache-stats': { destination: 'usage', tab: 'cache' },
    routing: { destination: 'advanced', tab: 'routing' },
    experiments: { destination: 'advanced', tab: 'experiments' },
    learning: { destination: 'advanced', tab: 'evaluation' },
    trajectory: { destination: 'advanced', tab: 'evaluation' },
    rollback: { destination: 'operations', tab: 'checkpoints' },
    'session-search': { destination: 'chat', tab: 'search' },
    'channel-config': { destination: 'channels', tab: 'setup' },
    'remote-access': { destination: 'operations', tab: 'remote-access' },
};

export function resolveAgentRoute(page: ThinClawPage): ResolvedAgentRoute {
    return AGENT_ROUTE_ALIASES[page];
}

export function agentRouteForDestination(destination: AgentPrimaryPage): AgentRouteDefinition {
    const route = AGENT_ROUTE_REGISTRY.find((candidate) => candidate.id === destination);
    if (!route) throw new Error(`Unknown Agent Cockpit destination: ${destination}`);
    return route;
}

export function isPrimaryAgentPage(page: ThinClawPage): page is AgentPrimaryPage {
    return (AGENT_PRIMARY_PAGE_IDS as readonly string[]).includes(page);
}
