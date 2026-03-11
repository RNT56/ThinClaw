import { useState, useEffect, useRef } from 'react';
import { useChatLayout } from '../ChatProvider';
import { OpenClawChatView } from '../../openclaw/OpenClawChatView';
import { OpenClawDashboard } from '../../openclaw/OpenClawDashboard';
import { OpenClawChannels } from '../../openclaw/OpenClawChannels';
import { OpenClawChannelStatus } from '../../openclaw/OpenClawChannelStatus';
import { OpenClawPresence } from '../../openclaw/OpenClawPresence';
import { OpenClawAutomations } from '../../openclaw/OpenClawAutomations';
import { OpenClawRoutineAudit } from '../../openclaw/OpenClawRoutineAudit';
import { OpenClawSkills } from '../../openclaw/OpenClawSkills';
import { OpenClawHooks } from '../../openclaw/OpenClawHooks';
import { OpenClawPlugins } from '../../openclaw/OpenClawPlugins';
import { OpenClawConfig } from '../../openclaw/OpenClawConfig';
import { OpenClawDoctor } from '../../openclaw/OpenClawDoctor';
import { OpenClawEventInspector } from '../../openclaw/OpenClawEventInspector';
import { OpenClawToolPolicies } from '../../openclaw/OpenClawToolPolicies';
import { OpenClawPairing } from '../../openclaw/OpenClawPairing';
import { OpenClawSystemControl } from '../../openclaw/OpenClawSystemControl';
import { OpenClawBrain } from '../../openclaw/OpenClawBrain';
import { OpenClawMemory } from '../../openclaw/OpenClawMemory';
import { FleetCommandCenter } from '../../openclaw/fleet/FleetCommandCenter';
import { OpenClawCostDashboard } from '../../openclaw/OpenClawCostDashboard';
import { OpenClawCacheStats } from '../../openclaw/OpenClawCacheStats';
import { OpenClawRouting } from '../../openclaw/OpenClawRouting';
import * as openclaw from '../../../lib/openclaw';

export function OpenClawView() {
    const {
        activeOpenClawPage,
        selectedOpenClawSession,
        openclawGatewayRunning,
        setSelectedOpenClawSession,
        setActiveOpenClawPage,
        setActiveTab,
    } = useChatLayout();

    // Track whether bootstrap check has been performed this session
    const bootstrapCheckedRef = useRef(false);
    const [bootstrapNeeded, setBootstrapNeeded] = useState<boolean | null>(null);

    const checkBootstrap = () => {
        openclaw.checkBootstrapNeeded()
            .then(needed => {
                setBootstrapNeeded(needed);
                // If bootstrap needed, navigate straight to chat — the agent will lead
                if (needed) {
                    setActiveOpenClawPage('chat');
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
    }, [setActiveOpenClawPage]);

    // Render the active non-chat sub-page (these don't need state preservation)
    const renderSubPage = () => {
        switch (activeOpenClawPage) {
            case 'chat': return null; // Chat is always-mounted below
            case 'dashboard': return <OpenClawDashboard />;
            case 'fleet': return <FleetCommandCenter />;
            case 'channels': return <OpenClawChannels />;
            case 'channel-status': return <OpenClawChannelStatus />;
            case 'presence': return <OpenClawPresence />;
            case 'automations': return <OpenClawAutomations />;
            case 'routine-audit': return <OpenClawRoutineAudit />;
            case 'skills': return <OpenClawSkills />;
            case 'hooks': return <OpenClawHooks />;
            case 'plugins': return <OpenClawPlugins />;
            case 'system-control': return <OpenClawSystemControl />;
            case 'brain': return <OpenClawBrain />;
            case 'memory': return <OpenClawMemory />;
            case 'config': return <OpenClawConfig />;
            case 'doctor': return <OpenClawDoctor />;
            case 'event-inspector': return <OpenClawEventInspector />;
            case 'tool-policies': return <OpenClawToolPolicies />;
            case 'pairing': return <OpenClawPairing />;
            case 'cost-dashboard': return <OpenClawCostDashboard />;
            case 'cache-stats': return <OpenClawCacheStats />;
            case 'routing': return <OpenClawRouting />;
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
                style={{ display: activeOpenClawPage === 'chat' ? undefined : 'none' }}
            >
                <OpenClawChatView
                    sessionKey={selectedOpenClawSession}
                    gatewayRunning={openclawGatewayRunning}
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
                            openclaw.startOpenClawGateway().catch(() => { });
                        }, 2000);
                    }}
                    onNavigateToSettings={(page) => setActiveTab(page as any)}
                    onViewSession={(key) => {
                        setSelectedOpenClawSession(key);
                        setActiveOpenClawPage('chat');
                    }}
                />
            </div>

            {/* Other sub-pages — conditionally rendered (no critical state to preserve) */}
            {activeOpenClawPage !== 'chat' && renderSubPage()}
        </div>
    );
}
