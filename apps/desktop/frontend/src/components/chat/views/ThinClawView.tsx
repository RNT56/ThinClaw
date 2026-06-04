import { useState, useEffect, useRef } from 'react';
import { useChatLayout } from '../ChatProvider';
import { ThinClawChatView } from '../../thinclaw/ThinClawChatView';
import { ThinClawDashboard } from '../../thinclaw/ThinClawDashboard';
import { ThinClawChannels } from '../../thinclaw/ThinClawChannels';
import { ThinClawChannelStatus } from '../../thinclaw/ThinClawChannelStatus';
import { ThinClawPresence } from '../../thinclaw/ThinClawPresence';
import { ThinClawAutomations } from '../../thinclaw/ThinClawAutomations';
import { ThinClawJobs } from '../../thinclaw/ThinClawJobs';
import { ThinClawAutonomy } from '../../thinclaw/ThinClawAutonomy';
import { ThinClawRoutineAudit } from '../../thinclaw/ThinClawRoutineAudit';
import { ThinClawSkills } from '../../thinclaw/ThinClawSkills';
import { ThinClawHooks } from '../../thinclaw/ThinClawHooks';
import { ThinClawPlugins } from '../../thinclaw/ThinClawPlugins';
import { ThinClawConfig } from '../../thinclaw/ThinClawConfig';
import { ThinClawDoctor } from '../../thinclaw/ThinClawDoctor';
import { ThinClawEventInspector } from '../../thinclaw/ThinClawEventInspector';
import { ThinClawToolPolicies } from '../../thinclaw/ThinClawToolPolicies';
import { ThinClawPairing } from '../../thinclaw/ThinClawPairing';
import { ThinClawSystemControl } from '../../thinclaw/ThinClawSystemControl';
import { ThinClawBrain } from '../../thinclaw/ThinClawBrain';
import { ThinClawMemory } from '../../thinclaw/ThinClawMemory';
import { FleetCommandCenter } from '../../thinclaw/fleet/FleetCommandCenter';
import { ThinClawCostDashboard } from '../../thinclaw/ThinClawCostDashboard';
import { ThinClawCacheStats } from '../../thinclaw/ThinClawCacheStats';
import { ThinClawRouting } from '../../thinclaw/ThinClawRouting';
import { ThinClawExperiments } from '../../thinclaw/ThinClawExperiments';
import { ThinClawLearning } from '../../thinclaw/ThinClawLearning';
import * as thinclaw from '../../../lib/thinclaw';

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
            case 'presence': return <ThinClawPresence />;
            case 'automations': return <ThinClawAutomations />;
            case 'jobs': return <ThinClawJobs />;
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
            </div>

            {/* Other sub-pages — conditionally rendered (no critical state to preserve) */}
            {activeThinClawPage !== 'chat' && renderSubPage()}
        </div>
    );
}
