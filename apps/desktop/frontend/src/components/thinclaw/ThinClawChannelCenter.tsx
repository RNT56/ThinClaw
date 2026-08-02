import { lazy, Suspense } from 'react';

import { AsyncState } from '../ui';
import { AgentTabbedPage } from './AgentTabbedPage';

const ThinClawChannels = lazy(() => import('./ThinClawChannels').then((module) => ({ default: module.ThinClawChannels })));
const ThinClawChannelStatus = lazy(() => import('./ThinClawChannelStatus').then((module) => ({ default: module.ThinClawChannelStatus })));
const ThinClawChannelConfig = lazy(() => import('./ThinClawChannelConfig').then((module) => ({ default: module.ThinClawChannelConfig })));
const ThinClawPairing = lazy(() => import('./ThinClawPairing').then((module) => ({ default: module.ThinClawPairing })));

function LoadingChannels() {
    return <AsyncState kind="loading" title="Loading channels" className="flex-1" />;
}

export function ThinClawChannelCenter({ initialTab }: { initialTab?: string }) {
    return (
        <AgentTabbedPage
            eyebrow="Inbound and outbound agents"
            title="Channels"
            description="Discover live channel health, complete supported setup, and manage pairing without changing primary destinations."
            initialTab={initialTab}
            tabs={[
                { id: 'overview', label: 'Overview', content: <Suspense fallback={<LoadingChannels />}><ThinClawChannels /></Suspense> },
                { id: 'health', label: 'Health', content: <Suspense fallback={<LoadingChannels />}><ThinClawChannelStatus /></Suspense> },
                { id: 'setup', label: 'Setup', content: <Suspense fallback={<LoadingChannels />}><ThinClawChannelConfig /></Suspense> },
                { id: 'security', label: 'Security & pairing', content: <Suspense fallback={<LoadingChannels />}><ThinClawPairing /></Suspense> },
            ]}
        />
    );
}
