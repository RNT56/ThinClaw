import { useState, useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';

// Types for consumer
export interface StreamApproval {
    id: string;
    tool: string;
    input: any;
    status: 'pending' | 'approved' | 'denied';
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
    status: 'running' | 'completed' | 'failed' | 'idle';
    startedAt: number;
    completedAt?: number;
}

export function useOpenClawStream(sessionKey: string | null) {
    const [runs, setRuns] = useState<Record<string, StreamRun>>({});

    useEffect(() => {
        if (!sessionKey) return;

        console.log(`[useOpenClawStream] Subscribing to events for session: ${sessionKey}`);

        const unlistenPromise = listen<any>('clawdbot-event', (event) => {
            const uiEvent = event.payload;
            console.log('[useOpenClawStream] Received:', uiEvent.kind, uiEvent);

            // Only process events for the current session
            if (uiEvent.session_key !== sessionKey) return;

            const runId = uiEvent.run_id || 'default';

            setRuns(prev => {
                const currentRun = prev[runId] || {
                    id: runId,
                    text: "",
                    tools: [],
                    approvals: [],
                    status: "running",
                    startedAt: Date.now()
                };

                let updatedRun = { ...currentRun };

                switch (uiEvent.kind) {
                    case 'AssistantDelta':
                        updatedRun.text += uiEvent.delta;
                        break;
                    case 'AssistantSnapshot':
                    case 'AssistantInternal':
                        const prefix = uiEvent.kind === 'AssistantInternal' ? '🧠 ' : '';
                        updatedRun.text = prefix + uiEvent.text;
                        break;
                    case 'AssistantFinal':
                        updatedRun.text = uiEvent.text;
                        updatedRun.status = 'completed';
                        updatedRun.completedAt = Date.now();
                        break;
                    case 'ToolUpdate':
                        // Search for existing tool or add new one
                        const toolIdx = updatedRun.tools.findIndex(t => t.tool === uiEvent.tool_name && t.status === 'started');
                        if (toolIdx >= 0) {
                            updatedRun.tools = [...updatedRun.tools];
                            updatedRun.tools[toolIdx] = {
                                ...updatedRun.tools[toolIdx],
                                status: uiEvent.status === 'ok' ? 'completed' :
                                    uiEvent.status === 'error' ? 'failed' : 'running',
                                input: uiEvent.input || updatedRun.tools[toolIdx].input,
                                output: uiEvent.output || updatedRun.tools[toolIdx].output,
                            };
                        } else {
                            updatedRun.tools = [...updatedRun.tools, {
                                tool: uiEvent.tool_name,
                                input: uiEvent.input,
                                output: uiEvent.output,
                                status: uiEvent.status === 'ok' ? 'completed' :
                                    uiEvent.status === 'error' ? 'failed' : (uiEvent.status === 'started' ? 'started' : 'running'),
                                timestamp: Date.now()
                            }];
                        }
                        break;
                    case 'ApprovalRequested':
                        // Don't add duplicate approvals
                        if (!updatedRun.approvals.some(a => a.id === uiEvent.approval_id)) {
                            updatedRun.approvals = [...updatedRun.approvals, {
                                id: uiEvent.approval_id,
                                tool: uiEvent.tool_name,
                                input: uiEvent.input,
                                status: 'pending'
                            }];
                        }
                        break;
                    case 'ApprovalResolved':
                        updatedRun.approvals = updatedRun.approvals.map(a =>
                            a.id === uiEvent.approval_id
                                ? { ...a, status: uiEvent.approved ? 'approved' : 'denied' }
                                : a
                        );
                        break;
                    case 'RunStatus':
                        updatedRun.status = uiEvent.status === 'ok' ? 'completed' :
                            uiEvent.status === 'error' ? 'failed' :
                                uiEvent.status === 'aborted' ? 'failed' : 'running';
                        if (updatedRun.status === 'completed' || updatedRun.status === 'failed') {
                            updatedRun.completedAt = Date.now();
                        }
                        break;
                }

                return { ...prev, [runId]: updatedRun };
            });
        });

        return () => {
            unlistenPromise.then(fn => fn());
        };
    }, [sessionKey]);

    return { isConnected: true, lastError: null, runs };
}
