import { lazy, Suspense } from 'react';

import { AsyncState } from '../ui';
import { useAgentCockpit } from './AgentCockpitProvider';
import { AgentTabbedPage } from './AgentTabbedPage';

const ThinClawBrain = lazy(() => import('./ThinClawBrain').then((module) => ({ default: module.ThinClawBrain })));
const ThinClawMemory = lazy(() => import('./ThinClawMemory').then((module) => ({ default: module.ThinClawMemory })));

function LoadingWorkspace() {
    return <AsyncState kind="loading" title="Loading workspace" className="flex-1" />;
}

export function ThinClawWorkspaceMemory({ initialTab }: { initialTab?: string }) {
    const { capability } = useAgentCockpit();
    const localFilesCapability = capability('local-host');
    return (
        <AgentTabbedPage
            eyebrow="Agent context"
            title="Workspace & Memory"
            description="Agent-owned documents and persistent memory. Local host files are visibly unavailable while operating a remote profile."
            initialTab={initialTab}
            tabs={[
                {
                    id: 'workspace',
                    label: 'Agent workspace',
                    content: <Suspense fallback={<LoadingWorkspace />}><ThinClawBrain localFilesCapability={localFilesCapability} /></Suspense>,
                },
                {
                    id: 'memory',
                    label: 'Daily memory & search',
                    content: <Suspense fallback={<LoadingWorkspace />}><ThinClawMemory /></Suspense>,
                },
            ]}
        />
    );
}
