import { lazy, Suspense } from 'react';

import { AsyncState } from '../ui';
import { AgentTabbedPage } from './AgentTabbedPage';

const ThinClawAutomations = lazy(() => import('./ThinClawAutomations').then((module) => ({ default: module.ThinClawAutomations })));
const ThinClawRoutineAudit = lazy(() => import('./ThinClawRoutineAudit').then((module) => ({ default: module.ThinClawRoutineAudit })));

function LoadingAutomations() {
    return <AsyncState kind="loading" title="Loading automations" className="flex-1" />;
}

export function ThinClawAutomationCenter({ initialTab }: { initialTab?: string }) {
    return (
        <AgentTabbedPage
            eyebrow="Background work"
            title="Automations"
            description="Create, run, and inspect scheduled or event-triggered agent routines in one workflow."
            initialTab={initialTab}
            tabs={[
                { id: 'automations', label: 'Automations', content: <Suspense fallback={<LoadingAutomations />}><ThinClawAutomations /></Suspense> },
                { id: 'history', label: 'History', content: <Suspense fallback={<LoadingAutomations />}><ThinClawRoutineAudit /></Suspense> },
            ]}
        />
    );
}
