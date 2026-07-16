import type { ThinClawMessage } from './thinclaw';

export interface ThinClawTimelineGroup {
    type: 'msg' | 'group';
    items: ThinClawMessage[];
}

export interface ThinClawTimelineItem {
    ts: number;
    index: number;
    data: ThinClawTimelineGroup;
}

function isVisibleChatMessage(message: ThinClawMessage): boolean {
    if (message.text.includes('🧠')) return false;
    if (message.text.includes('HEARTBEAT_POLL')) return false;
    if (message.text.includes('SYSTEM_CONTEXT_REFRESH')) return false;
    if (message.text.includes('[SYSTEM_CONTEXT_UPDATE]')) return false;
    if (message.text.trim().startsWith('[Tool Call:')) return false;
    if (message.text.includes('Pre-compaction memory flush')) return false;
    if (message.text.includes('Store durable memories now')) return false;
    if (message.text.includes('NO_REPL')) return false;
    if (message.role === 'system') return false;

    if (message.role === 'assistant' && message.text.includes('[TOOL_CALLS]')) {
        const withoutToolCalls = message.text
            .replace(/\[TOOL_CALLS\]\w+\[ARGS\]\{.*?\}[\s]*/gm, '')
            .trim();
        if (!withoutToolCalls) return false;
    }

    return message.role === 'user' || message.role === 'assistant';
}

export function buildThinClawTimeline(
    messages: ThinClawMessage[],
    chatOnly: boolean,
): ThinClawTimelineItem[] {
    const groups: ThinClawTimelineGroup[] = [];
    let currentSystemGroup: ThinClawMessage[] = [];

    for (const message of messages) {
        if (message.text.trim().startsWith('NO_REPL')) continue;
        if (message.text.trim().length === 0 && message.role !== 'system') continue;

        const isProgressEvent = message.metadata?.type === 'plan' || message.metadata?.type === 'usage';
        const isSystemTool = (message.role === 'system' && !isProgressEvent)
            || message.metadata?.type === 'tool'
            || message.text.includes('[Tool');
        const isTool = isSystemTool && !message.text.includes('🧠');

        if (isTool) {
            currentSystemGroup.push(message);
            continue;
        }
        if (currentSystemGroup.length > 0) {
            groups.push({ type: 'group', items: currentSystemGroup });
            currentSystemGroup = [];
        }
        groups.push({ type: 'msg', items: [message] });
    }
    if (currentSystemGroup.length > 0) groups.push({ type: 'group', items: currentSystemGroup });

    return groups
        .map((data, index) => ({ data, index, ts: data.items[0]?.ts_ms ?? 0 }))
        .sort((left, right) => left.ts - right.ts || left.index - right.index)
        .filter((item) => !chatOnly || (
            item.data.type === 'msg' && isVisibleChatMessage(item.data.items[0])
        ));
}
