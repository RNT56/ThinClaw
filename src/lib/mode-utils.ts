export type OpenClawMode = 'A1' | 'A2' | 'A3' | 'B1' | 'B2' | 'B3';

export interface ModeInfo {
    code: OpenClawMode;
    title: string;
    description: string;
    safe: boolean;
}

export function determineOperationMode(
    gatewayMode: string,
    nodeHostEnabled: boolean,
    localInferenceEnabled: boolean
): ModeInfo {
    const isRemote = gatewayMode === 'remote';

    if (!isRemote) {
        // Local Gateway Modes (Class A)
        if (nodeHostEnabled) {
            return {
                code: 'A3',
                title: 'Full Local Assistant',
                description: 'Local Brain • Local Body • Full Automation',
                safe: false // Automation enabled
            };
        }
        if (localInferenceEnabled) {
            return {
                code: 'A2',
                title: 'Local LLM App',
                description: 'Local Brain • No Automation • Local Intelligence',
                safe: true
            };
        }
        return {
            code: 'A1',
            title: 'Chat UI Only',
            description: 'Local Brain • No Automation • Cloud Models Only',
            safe: true
        };
    } else {
        // Remote Gateway Modes (Class B)
        if (nodeHostEnabled) {
            return {
                code: 'B3',
                title: 'Remote Drone',
                description: 'Remote Brain • Local Body • Remote Control Active',
                safe: false // Vulnerable to remote control
            };
        }
        if (localInferenceEnabled) {
            return {
                code: 'B2',
                title: 'Compute Node',
                description: 'Remote Brain • No Automation • Local Compute Shared',
                safe: true // Mostly safe, but sharing compute
            };
        }
        return {
            code: 'B1',
            title: 'Safe Client',
            description: 'Remote Brain • No Automation • Pure Interface',
            safe: true
        };
    }
}
