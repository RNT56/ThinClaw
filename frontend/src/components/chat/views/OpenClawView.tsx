import { useChatLayout } from '../ChatProvider';
import { OpenClawChatView } from '../../openclaw/OpenClawChatView';
import { OpenClawDashboard } from '../../openclaw/OpenClawDashboard';
import { OpenClawChannels } from '../../openclaw/OpenClawChannels';
import { OpenClawPresence } from '../../openclaw/OpenClawPresence';
import { OpenClawAutomations } from '../../openclaw/OpenClawAutomations';
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

export function OpenClawView() {
    const {
        activeOpenClawPage,
        selectedOpenClawSession,
        openclawGatewayRunning,
        setSelectedOpenClawSession,
        setActiveOpenClawPage,
        setActiveTab,
    } = useChatLayout();

    return (
        <div className="flex-1 flex flex-col h-full overflow-hidden">
            {activeOpenClawPage === 'chat' ? (
                <OpenClawChatView
                    sessionKey={selectedOpenClawSession}
                    gatewayRunning={openclawGatewayRunning}
                    onNavigateToSettings={(page) => setActiveTab(page as any)}
                    onViewSession={(key) => {
                        setSelectedOpenClawSession(key);
                        setActiveOpenClawPage('chat');
                    }}
                />
            ) : activeOpenClawPage === 'dashboard' ? (
                <OpenClawDashboard />
            ) : activeOpenClawPage === 'fleet' ? (
                <FleetCommandCenter />
            ) : activeOpenClawPage === 'channels' ? (
                <OpenClawChannels />
            ) : activeOpenClawPage === 'presence' ? (
                <OpenClawPresence />
            ) : activeOpenClawPage === 'automations' ? (
                <OpenClawAutomations />
            ) : activeOpenClawPage === 'skills' ? (
                <OpenClawSkills />
            ) : activeOpenClawPage === 'hooks' ? (
                <OpenClawHooks />
            ) : activeOpenClawPage === 'plugins' ? (
                <OpenClawPlugins />
            ) : activeOpenClawPage === 'system-control' ? (
                <OpenClawSystemControl />
            ) : activeOpenClawPage === 'brain' ? (
                <OpenClawBrain />
            ) : activeOpenClawPage === 'memory' ? (
                <OpenClawMemory />
            ) : activeOpenClawPage === 'config' ? (
                <OpenClawConfig />
            ) : activeOpenClawPage === 'doctor' ? (
                <OpenClawDoctor />
            ) : activeOpenClawPage === 'event-inspector' ? (
                <OpenClawEventInspector />
            ) : activeOpenClawPage === 'tool-policies' ? (
                <OpenClawToolPolicies />
            ) : activeOpenClawPage === 'pairing' ? (
                <OpenClawPairing />
            ) : (
                <div className="flex-1 flex items-center justify-center text-muted-foreground">
                    Select a page from the sidebar.
                </div>
            )}
        </div>
    );
}
