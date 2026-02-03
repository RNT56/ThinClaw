
// OpenClaw Gateway Protocol Types
// Based on protocol.schema.json

export interface GatewayFrame {
    type: "req" | "res" | "event";
    id?: string;
    method?: string;
    params?: any;
    ok?: boolean;
    payload?: any;
    error?: any;
    event?: string;
    seq?: number;
    stateVersion?: any;
}

export interface ConnectParams {
    minProtocol: number;
    maxProtocol: number;
    client: {
        id: string;
        displayName: string;
        version: string;
        platform: string;
        mode: string;
        instanceId: string;
    };
    auth?: {
        token?: string;
    };
}

export interface AgentEvent {
    runId: string;
    stream: "assistant" | "tool" | "lifecycle";
    data: any;
}

export interface ToolEventData {
    tool: string;
    input?: any;
    output?: any;
    error?: any;
    timestamp: number;
}

export interface LifecycleEventData {
    phase: "start" | "end" | "error";
    timestamp: number;
}

export interface AssistantEventData {
    delta: string;
    timestamp: number;
}
