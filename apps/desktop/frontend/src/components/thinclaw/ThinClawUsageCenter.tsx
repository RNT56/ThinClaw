import { lazy, Suspense } from 'react';

import { AsyncState } from '../ui';
import { AgentTabbedPage } from './AgentTabbedPage';

const ThinClawCostDashboard = lazy(() => import('./ThinClawCostDashboard').then((module) => ({ default: module.ThinClawCostDashboard })));
const ThinClawCacheStats = lazy(() => import('./ThinClawCacheStats').then((module) => ({ default: module.ThinClawCacheStats })));

function LoadingUsage() {
    return <AsyncState kind="loading" title="Loading usage" className="flex-1" />;
}

export function ThinClawUsageCenter({ initialTab }: { initialTab?: string }) {
    return (
        <AgentTabbedPage
            eyebrow="Evidence and limits"
            title="Usage"
            description="Review recorded agent cost and cache evidence. Usage is profile data, not a provider billing statement."
            initialTab={initialTab}
            tabs={[
                { id: 'cost', label: 'Cost', content: <Suspense fallback={<LoadingUsage />}><ThinClawCostDashboard /></Suspense> },
                { id: 'cache', label: 'Cache', content: <Suspense fallback={<LoadingUsage />}><ThinClawCacheStats /></Suspense> },
            ]}
        />
    );
}
