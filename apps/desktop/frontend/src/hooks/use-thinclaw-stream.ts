import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef } from 'react';

import type { UiEvent } from '../lib/bindings';

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

export type ThinClawEventHandler = (event: UiEvent) => void;

const eventSubscribers = new Set<ThinClawEventHandler>();
let eventBusStart: Promise<void> | null = null;

/**
 * Start the one native `thinclaw-event` listener for the frontend process.
 * React consumers subscribe to the in-process fan-out below, so mounting a
 * panel never creates a second Tauri listener or invents an untyped event view.
 */
function ensureThinClawEventBus(): Promise<void> {
    if (!eventBusStart) {
        eventBusStart = listen<UiEvent>('thinclaw-event', ({ payload }) => {
            for (const subscriber of [...eventSubscribers]) {
                try {
                    subscriber(payload);
                } catch (error) {
                    console.error('[thinclaw-event] subscriber failed:', error);
                }
            }
        }).then(() => undefined).catch((error) => {
            eventBusStart = null;
            throw error;
        });
    }
    return eventBusStart;
}

/** Subscribe imperative consumers through the shared typed event bus. */
export function subscribeThinClawEvents(handler: ThinClawEventHandler): () => void {
    eventSubscribers.add(handler);
    void ensureThinClawEventBus().catch((error) => {
        console.error('[thinclaw-event] failed to start event bus:', error);
    });
    return () => {
        eventSubscribers.delete(handler);
    };
}

/** React subscription to the generated `UiEvent` discriminated union. */
export function useThinClawEvents(handler: ThinClawEventHandler, enabled = true): void {
    const handlerRef = useRef(handler);
    handlerRef.current = handler;

    useEffect(() => {
        if (!enabled) return undefined;
        return subscribeThinClawEvents((event) => handlerRef.current(event));
    }, [enabled]);
}

export type { UiEvent } from '../lib/bindings';
