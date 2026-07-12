import { lazy, Suspense, useState, useEffect, useRef } from 'react';
import { useChatLayout } from '../ChatProvider';
import * as thinclaw from '../../../lib/thinclaw';

const ThinClawChatView = lazy(() => import('../../thinclaw/ThinClawChatView').then((module) => ({ default: module.ThinClawChatView })));
const ThinClawDashboard = lazy(() => import('../../thinclaw/ThinClawDashboard').then((module) => ({ default: module.ThinClawDashboard })));
const ThinClawChannels = lazy(() => import('../../thinclaw/ThinClawChannels').then((module) => ({ default: module.ThinClawChannels })));
const ThinClawChannelStatus = lazy(() => import('../../thinclaw/ThinClawChannelStatus').then((module) => ({ default: module.ThinClawChannelStatus })));
const ThinClawChannelConfig = lazy(() => import('../../thinclaw/ThinClawChannelConfig').then((module) => ({ default: module.ThinClawChannelConfig })));
const ThinClawPresence = lazy(() => import('../../thinclaw/ThinClawPresence').then((module) => ({ default: module.ThinClawPresence })));
const ThinClawAutomations = lazy(() => import('../../thinclaw/ThinClawAutomations').then((module) => ({ default: module.ThinClawAutomations })));
const ThinClawJobs = lazy(() => import('../../thinclaw/ThinClawJobs').then((module) => ({ default: module.ThinClawJobs })));
const ThinClawAutonomy = lazy(() => import('../../thinclaw/ThinClawAutonomy').then((module) => ({ default: module.ThinClawAutonomy })));
const ThinClawRoutineAudit = lazy(() => import('../../thinclaw/ThinClawRoutineAudit').then((module) => ({ default: module.ThinClawRoutineAudit })));
const ThinClawSkills = lazy(() => import('../../thinclaw/ThinClawSkills').then((module) => ({ default: module.ThinClawSkills })));
const ThinClawHooks = lazy(() => import('../../thinclaw/ThinClawHooks').then((module) => ({ default: module.ThinClawHooks })));
const ThinClawPlugins = lazy(() => import('../../thinclaw/ThinClawPlugins').then((module) => ({ default: module.ThinClawPlugins })));
const ThinClawConfig = lazy(() => import('../../thinclaw/ThinClawConfig').then((module) => ({ default: module.ThinClawConfig })));
const ThinClawDoctor = lazy(() => import('../../thinclaw/ThinClawDoctor').then((module) => ({ default: module.ThinClawDoctor })));
const ThinClawEventInspector = lazy(() => import('../../thinclaw/ThinClawEventInspector').then((module) => ({ default: module.ThinClawEventInspector })));
const ThinClawToolPolicies = lazy(() => import('../../thinclaw/ThinClawToolPolicies').then((module) => ({ default: module.ThinClawToolPolicies })));
const ThinClawPairing = lazy(() => import('../../thinclaw/ThinClawPairing').then((module) => ({ default: module.ThinClawPairing })));
const ThinClawSystemControl = lazy(() => import('../../thinclaw/ThinClawSystemControl').then((module) => ({ default: module.ThinClawSystemControl })));
const ThinClawBrain = lazy(() => import('../../thinclaw/ThinClawBrain').then((module) => ({ default: module.ThinClawBrain })));
const ThinClawMemory = lazy(() => import('../../thinclaw/ThinClawMemory').then((module) => ({ default: module.ThinClawMemory })));
const FleetCommandCenter = lazy(() => import('../../thinclaw/fleet/FleetCommandCenter').then((module) => ({ default: module.FleetCommandCenter })));
const ThinClawCostDashboard = lazy(() => import('../../thinclaw/ThinClawCostDashboard').then((module) => ({ default: module.ThinClawCostDashboard })));
const ThinClawCacheStats = lazy(() => import('../../thinclaw/ThinClawCacheStats').then((module) => ({ default: module.ThinClawCacheStats })));
const ThinClawTrajectory = lazy(() => import('../../thinclaw/ThinClawTrajectory').then((module) => ({ default: module.ThinClawTrajectory })));
const ThinClawRollback = lazy(() => import('../../thinclaw/ThinClawRollback').then((module) => ({ default: module.ThinClawRollback })));
const ThinClawSessionSearch = lazy(() => import('../../thinclaw/ThinClawSessionSearch').then((module) => ({ default: module.ThinClawSessionSearch })));
const ThinClawRouting = lazy(() => import('../../thinclaw/ThinClawRouting').then((module) => ({ default: module.ThinClawRouting })));
const ThinClawExperiments = lazy(() => import('../../thinclaw/ThinClawExperiments').then((module) => ({ default: module.ThinClawExperiments })));
const ThinClawLearning = lazy(() => import('../../thinclaw/ThinClawLearning').then((module) => ({ default: module.ThinClawLearning })));
const ThinClawRepoProjects = lazy(() => import('../../thinclaw/ThinClawRepoProjects').then((module) => ({ default: module.ThinClawRepoProjects })));

function ThinClawPageSkeleton() {
    return <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">Loading control surface…</div>;
}

export function ThinClawView() {
    const {
        activeThinClawPage,
        selectedThinClawSession,
        thinclawGatewayRunning,
        setSelectedThinClawSession,
        setActiveThinClawPage,
        setActiveTab,
    } = useChatLayout();

    // Track whether bootstrap check has been performed this session
    const bootstrapCheckedRef = useRef(false);
    const [bootstrapNeeded, setBootstrapNeeded] = useState<boolean | null>(null);

    const checkBootstrap = () => {
        thinclaw.checkBootstrapNeeded()
            .then(needed => {
                setBootstrapNeeded(needed);
                // If bootstrap needed, navigate straight to chat — the agent will lead
                if (needed) {
                    setActiveThinClawPage('chat');
                }
            })
            .catch(() => {
                setBootstrapNeeded(false);
            });
    };

    useEffect(() => {
        // Only check once on mount
        if (bootstrapCheckedRef.current) return;
        bootstrapCheckedRef.current = true;
        checkBootstrap();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [setActiveThinClawPage]);

    // Render the active non-chat sub-page (these don't need state preservation)
    const renderSubPage = () => {
        switch (activeThinClawPage) {
            case 'chat': return null; // Chat is always-mounted below
            case 'dashboard': return <ThinClawDashboard />;
            case 'fleet': return <FleetCommandCenter />;
            case 'channels': return <ThinClawChannels />;
            case 'channel-status': return <ThinClawChannelStatus />;
            case 'channel-config': return <ThinClawChannelConfig />;
            case 'presence': return <ThinClawPresence />;
            case 'automations': return <ThinClawAutomations />;
            case 'jobs': return <ThinClawJobs />;
            case 'repo-projects': return <ThinClawRepoProjects />;
            case 'autonomy': return <ThinClawAutonomy />;
            case 'routine-audit': return <ThinClawRoutineAudit />;
            case 'skills': return <ThinClawSkills />;
            case 'hooks': return <ThinClawHooks />;
            case 'plugins': return <ThinClawPlugins />;
            case 'system-control': return <ThinClawSystemControl />;
            case 'brain': return <ThinClawBrain />;
            case 'memory': return <ThinClawMemory />;
            case 'config': return <ThinClawConfig />;
            case 'doctor': return <ThinClawDoctor />;
            case 'event-inspector': return <ThinClawEventInspector />;
            case 'tool-policies': return <ThinClawToolPolicies />;
            case 'pairing': return <ThinClawPairing />;
            case 'cost-dashboard': return <ThinClawCostDashboard />;
            case 'cache-stats': return <ThinClawCacheStats />;
            case 'trajectory': return <ThinClawTrajectory />;
            case 'rollback': return <ThinClawRollback />;
            case 'session-search': return <ThinClawSessionSearch />;
            case 'routing': return <ThinClawRouting />;
            case 'experiments': return <ThinClawExperiments />;
            case 'learning': return <ThinClawLearning />;
            default: return (
                <div className="flex-1 flex items-center justify-center text-muted-foreground">
                    Select a page from the sidebar.
                </div>
            );
        }
    };

    return (
        <div className="flex-1 flex flex-col h-full overflow-hidden">
            {/* Chat view — always mounted to preserve messages, event listeners,
                and active run state. Hidden via CSS when another sub-page is active. */}
            <div
                className="flex-1 flex flex-col h-full overflow-hidden"
                style={{ display: activeThinClawPage === 'chat' ? undefined : 'none' }}
            >
                <Suspense fallback={<ThinClawPageSkeleton />}>
                    <ThinClawChatView
                        sessionKey={selectedThinClawSession}
                        gatewayRunning={thinclawGatewayRunning}
                        bootstrapNeeded={bootstrapNeeded ?? false}
                        onBootstrapComplete={() => setBootstrapNeeded(false)}
                        onFactoryReset={() => {
                            // Backend has reset bootstrap_completed=false in identity.json.
                            // Re-check so button label and auto-trigger update immediately.
                            bootstrapCheckedRef.current = false;
                            checkBootstrap();

                            // Auto-restart the gateway so the bootstrap ritual kicks off
                            // without requiring the user to manually click "Start Gateway".
                            // Small delay gives the DB deletion time to finish.
                            setTimeout(() => {
                                thinclaw.startThinClawGateway().catch(() => { });
                            }, 2000);
                        }}
                        onNavigateToSettings={(page) => setActiveTab(page as any)}
                        onViewSession={(key) => {
                            setSelectedThinClawSession(key);
                            setActiveThinClawPage('chat');
                        }}
                    />
                </Suspense>
            </div>

            {/* Other sub-pages — conditionally rendered (no critical state to preserve) */}
            {activeThinClawPage !== 'chat' && (
                <Suspense fallback={<ThinClawPageSkeleton />}>{renderSubPage()}</Suspense>
            )}
        </div>
    );
}
