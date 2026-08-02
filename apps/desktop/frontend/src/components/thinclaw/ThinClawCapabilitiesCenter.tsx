import { lazy, Suspense } from 'react';

import { AsyncState } from '../ui';
import { AgentTabbedPage } from './AgentTabbedPage';

const ThinClawSkills = lazy(() => import('./ThinClawSkills').then((module) => ({ default: module.ThinClawSkills })));
const ThinClawPlugins = lazy(() => import('./ThinClawPlugins').then((module) => ({ default: module.ThinClawPlugins })));
const ThinClawToolPolicies = lazy(() => import('./ThinClawToolPolicies').then((module) => ({ default: module.ThinClawToolPolicies })));
const ThinClawHooks = lazy(() => import('./ThinClawHooks').then((module) => ({ default: module.ThinClawHooks })));

function LoadingCapabilities() {
    return <AsyncState kind="loading" title="Loading capabilities" className="flex-1" />;
}

export function ThinClawCapabilitiesCenter({ initialTab }: { initialTab?: string }) {
    return (
        <AgentTabbedPage
            eyebrow="Agent capabilities"
            title="Capabilities"
            description="Inspect and manage the skills, extensions, tool availability, and advanced lifecycle hooks exposed by this profile."
            initialTab={initialTab}
            tabs={[
                { id: 'skills', label: 'Skills', content: <Suspense fallback={<LoadingCapabilities />}><ThinClawSkills /></Suspense> },
                { id: 'extensions', label: 'Extensions & MCP', content: <Suspense fallback={<LoadingCapabilities />}><ThinClawPlugins /></Suspense> },
                { id: 'tools', label: 'Tool access', content: <Suspense fallback={<LoadingCapabilities />}><ThinClawToolPolicies /></Suspense> },
                { id: 'hooks', label: 'Hooks (advanced)', content: <Suspense fallback={<LoadingCapabilities />}><ThinClawHooks /></Suspense> },
            ]}
        />
    );
}
