import { lazy, Suspense } from 'react';

import { AsyncState } from '../ui';
import { AgentTabbedPage } from './AgentTabbedPage';

const ThinClawSystemControl = lazy(() => import('./ThinClawSystemControl').then((module) => ({ default: module.ThinClawSystemControl })));
const ThinClawDoctor = lazy(() => import('./ThinClawDoctor').then((module) => ({ default: module.ThinClawDoctor })));
const ThinClawRemoteAccess = lazy(() => import('./ThinClawRemoteAccess').then((module) => ({ default: module.ThinClawRemoteAccess })));
const ThinClawRollback = lazy(() => import('./ThinClawRollback').then((module) => ({ default: module.ThinClawRollback })));

function LoadingOperations() {
    return <AsyncState kind="loading" title="Loading operations" className="flex-1" />;
}

export function ThinClawOperationsCenter({ initialTab }: { initialTab?: string }) {
    return (
        <AgentTabbedPage
            eyebrow="Operational controls"
            title="Operations & Safety"
            description="Safely operate the gateway, inspect diagnostics, control local remote access, and recover from checkpoints."
            initialTab={initialTab}
            tabs={[
                { id: 'gateway', label: 'Gateway & logs', content: <Suspense fallback={<LoadingOperations />}><ThinClawSystemControl /></Suspense> },
                { id: 'diagnostics', label: 'Diagnostics', content: <Suspense fallback={<LoadingOperations />}><ThinClawDoctor /></Suspense> },
                { id: 'remote-access', label: 'Remote access', capability: 'remote-access', content: <Suspense fallback={<LoadingOperations />}><ThinClawRemoteAccess /></Suspense> },
                { id: 'checkpoints', label: 'Checkpoints', content: <Suspense fallback={<LoadingOperations />}><ThinClawRollback /></Suspense> },
            ]}
        />
    );
}
