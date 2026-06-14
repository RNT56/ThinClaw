// Types for run tracking — consumed by LiveAgentStatus and ThinClawChatView

export interface StreamApproval {
    id: string;
    tool: string;
    input: any;
    status: 'pending' | 'approved' | 'denied';
}

export interface StreamCredentialPrompt {
    id: string;
    secretName: string;
    provider: string;
    reason: string;
    status: 'pending' | 'stored';
}

export interface StreamRun {
    id: string;
    text: string;
    tools: {
        tool: string;
        input?: any;
        output?: any;
        status: 'started' | 'running' | 'completed' | 'failed';
        timestamp: number;
    }[];
    approvals: StreamApproval[];
    /** Inline credential prompts emitted by the agent (masked-input cards). */
    credentialPrompts?: StreamCredentialPrompt[];
    status: 'running' | 'completed' | 'failed' | 'idle';
    error?: string;
    startedAt: number;
    completedAt?: number;
}
