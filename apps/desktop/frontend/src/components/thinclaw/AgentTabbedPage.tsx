import { type ReactNode, useEffect, useMemo, useState } from 'react';

import type { AgentCapabilityKey } from './agent-routes';
import { useAgentCockpit } from './AgentCockpitProvider';
import { AgentPageShell, CapabilityGate, Notice, StatusBadge, Tabs } from '../ui';

export interface AgentPageTab {
    id: string;
    label: string;
    capability?: AgentCapabilityKey;
    content: ReactNode;
}

export interface AgentTabbedPageProps {
    title: string;
    description: string;
    eyebrow?: string;
    tabs: readonly AgentPageTab[];
    initialTab?: string;
    actions?: ReactNode;
}

/**
 * Consistent destination frame for the consolidated Agent Cockpit pages.
 * Legacy components remain lazily mounted inside their new task-oriented tab.
 */
export function AgentTabbedPage({ title, description, eyebrow, tabs, initialTab, actions }: AgentTabbedPageProps) {
    const { source, checkedAt, capability } = useAgentCockpit();
    const initial = useMemo(() => tabs.some((tab) => tab.id === initialTab) ? initialTab! : tabs[0]?.id, [initialTab, tabs]);
    const [activeTab, setActiveTab] = useState(initial);

    useEffect(() => {
        if (initialTab && tabs.some((tab) => tab.id === initialTab)) setActiveTab(initialTab);
    }, [initialTab, tabs]);

    const active = tabs.find((tab) => tab.id === activeTab) ?? tabs[0];
    const activeCapability = capability(active?.capability ?? 'always');
    const allowRemediationContent = !active?.capability || active.capability === 'always' || active.capability === 'advanced';
    const sourceLabel = source === 'remote' ? 'Remote profile' : source === 'local' ? 'Local Core' : 'Checking profile';
    const freshness = checkedAt ? new Date(checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : undefined;

    return (
        <AgentPageShell
            title={title}
            description={description}
            eyebrow={eyebrow}
            badge={<StatusBadge status={activeCapability.state} label={`${sourceLabel}${freshness ? ` · ${freshness}` : ''}`} />}
            actions={actions}
            contentClassName="flex min-h-0 flex-1 flex-col gap-5"
        >
            <Tabs
                ariaLabel={`${title} sections`}
                tabs={tabs.map(({ id, label }) => ({ id, label }))}
                value={active?.id ?? ''}
                onValueChange={setActiveTab}
            />
            {allowRemediationContent && activeCapability.state !== 'available' && (
                <Notice tone={activeCapability.state === 'stale' ? 'warning' : 'info'} title="Profile status needs attention">
                    {[activeCapability.reason, activeCapability.remediation].filter(Boolean).join(' ')}
                </Notice>
            )}
            {active && (
                <section
                    id={`${active.id}-panel`}
                    role="tabpanel"
                    aria-labelledby={`${active.id}-tab`}
                    className="flex min-h-0 flex-1 flex-col"
                >
                    <CapabilityGate capability={activeCapability} allowContent={allowRemediationContent}>{active.content}</CapabilityGate>
                </section>
            )}
        </AgentPageShell>
    );
}
