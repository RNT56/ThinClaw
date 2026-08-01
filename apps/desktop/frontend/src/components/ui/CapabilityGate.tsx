import { type ReactNode } from 'react';

import type { AgentCapabilityState } from '../thinclaw/AgentCockpitProvider';
import { AsyncState } from './AsyncState';

export interface CapabilityGateProps {
    capability: AgentCapabilityState;
    children: ReactNode;
    title?: string;
    /** Keep a remediation or informational surface reachable while status is unavailable. */
    allowContent?: boolean;
}

/** Renders unavailable/stale capability state before a page can imply support. */
export function CapabilityGate({ capability, children, title = 'This capability is not available', allowContent = false }: CapabilityGateProps) {
    if (capability.state === 'available' || allowContent) return <>{children}</>;
    if (capability.state === 'loading') {
        return <AsyncState kind="loading" title="Checking capability" description="Reading the selected agent profile." />;
    }
    const stale = capability.state === 'stale';
    return (
        <AsyncState
            kind={stale ? 'stale' : 'unavailable'}
            title={stale ? 'Capability status is stale' : title}
            description={[capability.reason, capability.remediation].filter(Boolean).join(' ')}
        />
    );
}
