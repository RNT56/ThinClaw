import { lazy, Suspense } from 'react';
import { FlaskConical } from 'lucide-react';

import { AsyncState, Notice, Surface } from '../ui';
import { AgentTabbedPage } from './AgentTabbedPage';

const FleetCommandCenter = lazy(() => import('./fleet/FleetCommandCenter').then((module) => ({ default: module.FleetCommandCenter })));
const ThinClawRouting = lazy(() => import('./ThinClawRouting').then((module) => ({ default: module.ThinClawRouting })));
const ThinClawLearning = lazy(() => import('./ThinClawLearning').then((module) => ({ default: module.ThinClawLearning })));
const ThinClawAutonomy = lazy(() => import('./ThinClawAutonomy').then((module) => ({ default: module.ThinClawAutonomy })));
const ThinClawExperiments = lazy(() => import('./ThinClawExperiments').then((module) => ({ default: module.ThinClawExperiments })));
const ThinClawConfig = lazy(() => import('./ThinClawConfig').then((module) => ({ default: module.ThinClawConfig })));
const ThinClawEventInspector = lazy(() => import('./ThinClawEventInspector').then((module) => ({ default: module.ThinClawEventInspector })));

function LoadingLab() {
    return <AsyncState kind="loading" title="Loading advanced control" className="flex-1" />;
}

function LabsOverview() {
    return (
        <div className="mx-auto grid w-full max-w-5xl gap-4 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
            <Surface className="p-5">
                <div className="flex items-center gap-2">
                    <FlaskConical className="size-4 text-primary" aria-hidden="true" />
                    <h2 className="text-sm font-semibold">Specialist controls</h2>
                </div>
                <p className="mt-2 text-sm leading-relaxed text-content-muted">
                    These surfaces expose expert or experimental agent features. Availability varies by profile, runtime mode, host permissions, and backend support. Each panel must explain missing requirements before it enables a change.
                </p>
            </Surface>
            <Notice tone="warning" title="Use a deliberate workflow">
                Fleet dispatch is broadcast-only until a target profile can return a transport receipt. Repo Projects remains quarantined until an end-to-end live integration exists.
            </Notice>
        </div>
    );
}

function RepoProjectsQuarantine() {
    return (
        <div className="mx-auto w-full max-w-4xl">
            <Notice tone="warning" title="Repo Projects is not available as a live Desktop control surface">
                The previous page could render fabricated repositories, workers, pull requests, checks, and events. It is intentionally quarantined until the selected profile proves an enrolled repository, authenticated provider, dispatch transport, event provenance, pull request, checks, and merge-gate flow end to end.
            </Notice>
        </div>
    );
}

export function ThinClawAdvancedLabs({ initialTab }: { initialTab?: string }) {
    return (
        <AgentTabbedPage
            eyebrow="Specialist controls"
            title="Advanced / Labs"
            description="Experimental and expert-only controls. Verify profile, permissions, and consequences before making changes."
            initialTab={initialTab}
            tabs={[
                { id: 'overview', label: 'Overview', content: <LabsOverview /> },
                { id: 'fleet', label: 'Fleet', content: <Suspense fallback={<LoadingLab />}><FleetCommandCenter /></Suspense> },
                { id: 'routing', label: 'Routing', content: <Suspense fallback={<LoadingLab />}><ThinClawRouting /></Suspense> },
                { id: 'evaluation', label: 'Evaluation', content: <Suspense fallback={<LoadingLab />}><ThinClawLearning /></Suspense> },
                { id: 'autonomy', label: 'Autonomy', content: <Suspense fallback={<LoadingLab />}><ThinClawAutonomy /></Suspense> },
                { id: 'experiments', label: 'Experiments', content: <Suspense fallback={<LoadingLab />}><ThinClawExperiments /></Suspense> },
                { id: 'developer', label: 'Developer settings', content: <Suspense fallback={<LoadingLab />}><ThinClawConfig /></Suspense> },
                { id: 'events', label: 'Developer events', content: <Suspense fallback={<LoadingLab />}><ThinClawEventInspector /></Suspense> },
                { id: 'projects', label: 'Repo Projects', content: <RepoProjectsQuarantine /> },
            ]}
        />
    );
}
