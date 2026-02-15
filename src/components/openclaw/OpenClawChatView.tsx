import { invoke } from '@tauri-apps/api/core';

import { useState, useEffect, useCallback, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Send, Radio, RefreshCw, AlertTriangle, Clock, User, Bot, Settings, ChevronRight, ChevronDown, Brain, Terminal, Loader2, CheckCircle2, XCircle, Layers, Zap, ExternalLink, Trash2 } from 'lucide-react';
import { commands } from '../../lib/bindings';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import * as openclaw from '../../lib/openclaw';
import { OpenClawMessage } from '../../lib/openclaw';
import { listen } from '@tauri-apps/api/event';
import ReactMarkdown from 'react-markdown';

import { StreamRun } from '../../hooks/use-openclaw-stream';
import { LiveAgentStatus } from './LiveAgentStatus';
import { MemoryEditor } from './MemoryEditor';
import { Square } from 'lucide-react';

interface OpenClawChatViewProps {
    sessionKey: string | null;
    gatewayRunning: boolean;
    onNavigateToSettings?: (page: 'openclaw-gateway') => void;
    onViewSession?: (sessionKey: string) => void;
}

interface RichToolCardProps {
    name: string;
    status: 'started' | 'completed' | 'failed' | 'in_flight';
    input?: any;
    output?: any;
    isSubAgent?: boolean;
    variant?: 'live' | 'history';
    onViewSession?: (sessionKey: string) => void;
}

function RichToolCard({ name, status, input, output, isSubAgent, variant = 'live', onViewSession }: RichToolCardProps) {
    const [isExpanded, setIsExpanded] = useState(false);

    // Extract spawned session key from sub-agent output
    const spawnedSessionKey = isSubAgent ? extractSessionKey(output) : null;

    // Minimal History Mode
    if (variant === 'history') {
        return (
            <div className="w-full">
                <button
                    onClick={() => setIsExpanded(!isExpanded)}
                    className="flex items-center gap-2 w-full text-left py-1 hover:bg-white/5 transition-colors rounded group pl-2 border-l border-white/5"
                >
                    <div className="w-4 h-4 rounded flex items-center justify-center bg-gray-800/50 group-hover:bg-gray-800 transition-colors">
                        {isSubAgent ? <RefreshCw className="w-2.5 h-2.5 text-purple-400" /> : <Terminal className="w-2.5 h-2.5 text-gray-500" />}
                    </div>
                    <div className="flex-1 flex items-center gap-2">
                        <span className="text-[10px] font-medium text-muted-foreground group-hover:text-gray-300 transition-colors font-mono">
                            {isSubAgent ? 'Sub-Agent Spawn' : name}
                        </span>
                        {status === 'failed' && <span className="text-[9px] text-red-500 font-bold uppercase">Failed</span>}
                        {isSubAgent && status === 'started' && <span className="text-[9px] text-purple-400 font-bold uppercase animate-pulse">Running</span>}
                    </div>
                    {isExpanded ? <ChevronDown className="w-3 h-3 text-muted-foreground" /> : <ChevronRight className="w-3 h-3 text-muted-foreground" />}
                </button>
                <AnimatePresence>
                    {isExpanded && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: "auto", opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            className="overflow-hidden ml-7 space-y-2 mt-1 mb-2 border-l-2 border-white/5 pl-3"
                        >
                            {input && (
                                <div>
                                    <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-0.5">Input</div>
                                    <pre className="text-[10px] font-mono text-gray-400 overflow-x-auto whitespace-pre-wrap bg-black/20 p-2 rounded">
                                        {typeof input === 'string' ? input : JSON.stringify(input, null, 2)}
                                    </pre>
                                </div>
                            )}
                            {output && (
                                <div>
                                    <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-0.5">Output</div>
                                    <pre className="text-[10px] font-mono text-gray-500 overflow-x-auto whitespace-pre-wrap bg-black/20 p-2 rounded">
                                        {typeof output === 'string' ? output : JSON.stringify(output, null, 2)}
                                    </pre>
                                </div>
                            )}
                            {/* Sub-Agent Navigation Button */}
                            {spawnedSessionKey && onViewSession && (
                                <button
                                    onClick={(e) => { e.stopPropagation(); onViewSession(spawnedSessionKey); }}
                                    className="flex items-center gap-1.5 mt-1 px-2 py-1 bg-purple-500/10 hover:bg-purple-500/20 text-purple-400 text-[10px] font-bold border border-purple-500/20 rounded transition-all group/nav"
                                >
                                    <ExternalLink className="w-3 h-3 group-hover/nav:translate-x-0.5 transition-transform" />
                                    View Sub-Agent
                                </button>
                            )}
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>
        );
    }

    // Default Rich/Live Mode
    let StatusIcon = Terminal;
    let iconColor = "text-amber-500";
    let animate = false;

    if (isSubAgent) {
        StatusIcon = RefreshCw;
        iconColor = "text-purple-400";
    }

    if (status === 'started' || status === 'in_flight') {
        StatusIcon = Loader2;
        iconColor = "text-blue-400";
        animate = true;
    } else if (status === 'completed') {
        StatusIcon = CheckCircle2;
        iconColor = "text-green-400";
    } else if (status === 'failed') {
        StatusIcon = XCircle;
        iconColor = "text-red-400";
    }

    let label = isSubAgent ? "Sub-Agent Task" : `Action: ${name} `;
    if (status === 'started') label += " (Running...)";
    if (status === 'completed') label += " (Done)";
    if (status === 'failed') label += " (Failed)";

    return (
        <div className="w-full">
            <button
                onClick={() => setIsExpanded(!isExpanded)}
                className={cn(
                    "flex items-center gap-2 w-full text-left py-1 mb-1 transition-colors rounded hover:bg-white/5",
                    iconColor
                )}
            >
                <StatusIcon className={cn("w-3.5 h-3.5", animate && "animate-spin")} />
                <span className="text-[10px] font-bold uppercase tracking-wider flex-1">
                    {label}
                </span>
                {isExpanded ? <ChevronDown className="w-3 h-3 opacity-50" /> : <ChevronRight className="w-3 h-3 opacity-50" />}
            </button>
            <AnimatePresence>
                {isExpanded && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: "auto", opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden ml-6 space-y-2"
                    >
                        {input && (
                            <div className="bg-black/30 rounded p-2 border border-white/10">
                                <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-1">Input</div>
                                <pre className="text-[10px] font-mono text-gray-300 overflow-x-auto whitespace-pre-wrap">
                                    {typeof input === 'string' ? input : JSON.stringify(input, null, 2)}
                                </pre>
                            </div>
                        )}
                        {output && (
                            <div className="bg-black/30 rounded p-2 border border-white/10">
                                <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-1">Output</div>
                                {(() => {
                                    // Parse if string
                                    let content = output;
                                    if (typeof output === 'string' && output.trim().startsWith('{')) {
                                        try { content = JSON.parse(output); } catch { }
                                    }

                                    // Display error prominently
                                    if (content?.error || (content?.status === 'error' && content?.error)) {
                                        return (
                                            <div className="bg-red-500/10 border border-red-500/20 rounded p-2 text-red-400 text-xs font-mono whitespace-pre-wrap">
                                                {content.error}
                                            </div>
                                        );
                                    }

                                    return (
                                        <pre className="text-[10px] font-mono text-green-300/80 overflow-x-auto whitespace-pre-wrap">
                                            {typeof output === 'string' ? output : JSON.stringify(output, null, 2)}
                                        </pre>
                                    );
                                })()}
                            </div>
                        )}
                        {/* Sub-Agent Navigation Button (Live Mode) */}
                        {spawnedSessionKey && onViewSession && (
                            <button
                                onClick={(e) => { e.stopPropagation(); onViewSession(spawnedSessionKey); }}
                                className="flex items-center gap-2 mt-2 px-3 py-1.5 bg-purple-500/10 hover:bg-purple-500/20 text-purple-400 text-xs font-bold border border-purple-500/20 rounded-lg transition-all group/nav w-full justify-center"
                            >
                                <ExternalLink className="w-3.5 h-3.5 group-hover/nav:translate-x-0.5 transition-transform" />
                                View Sub-Agent Session
                            </button>
                        )}
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}

// Helper: extract session key from sub-agent spawn output
function extractSessionKey(output: any): string | null {
    if (!output) return null;
    let data = output;
    if (typeof output === 'string') {
        try { data = JSON.parse(output); } catch { return null; }
    }
    // Common patterns from sessions_spawn output
    return data?.sessionKey || data?.session_key || data?.sessionId || data?.session_id || null;
}

// Collapsed Group for History
function ToolHistoryGroup({ messages, onViewSession }: { messages: OpenClawMessage[]; onViewSession?: (sessionKey: string) => void }) {
    const [expanded, setExpanded] = useState(false);
    const count = messages.length;
    const hasFailures = messages.some(m => {
        // basic heuristic for failure
        return m.metadata?.status === 'failed' || m.text.includes('FAIL') || m.text.includes('error');
    });

    return (
        <div className="w-full my-2">
            <button
                onClick={() => setExpanded(!expanded)}
                className={cn(
                    "flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg transition-all border",
                    expanded ? "bg-white/5 border-white/10" : "bg-transparent border-transparent hover:bg-white/5",
                    "group"
                )}
            >
                <div className={cn(
                    "w-6 h-6 rounded-md flex items-center justify-center transition-colors",
                    hasFailures ? "bg-red-500/10 text-red-400" : "bg-blue-500/10 text-blue-400"
                )}>
                    {hasFailures ? <AlertTriangle className="w-3.5 h-3.5" /> : <Layers className="w-3.5 h-3.5" />}
                </div>
                <div className="flex-1">
                    <div className="flex items-center gap-2">
                        <span className="text-xs font-medium text-gray-300">
                            Executed {count} tool{count > 1 ? 's' : ''}
                        </span>
                        {hasFailures && <span className="text-[9px] uppercase font-bold text-red-500 bg-red-500/10 px-1.5 rounded">Issues Found</span>}
                    </div>
                </div>
                {expanded ? <ChevronDown className="w-4 h-4 text-gray-500" /> : <ChevronRight className="w-4 h-4 text-gray-500" />}
            </button>
            <AnimatePresence>
                {expanded && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: "auto", opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden pl-4 pr-1 py-2 space-y-1"
                    >
                        {messages.map((msg) => (
                            <SystemMessageContent key={msg.id} text={msg.text} metadata={msg.metadata} onViewSession={onViewSession} />
                        ))}
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}

function SystemMessageContent({ text, metadata, onViewSession }: { text: string; metadata?: any; onViewSession?: (sessionKey: string) => void }) {
    // 1. Rich Tool Card Support (Backend Metadata)
    if (metadata?.type === 'tool' || metadata?.type === 'tool_result') {
        const isSubAgent = metadata.name === 'sessions_spawn' || metadata.name?.includes('subagent');
        return (
            <RichToolCard
                name={metadata.name || 'Unknown Tool'}
                status={metadata.status || 'completed'}
                input={metadata.input}
                output={metadata.output}
                isSubAgent={isSubAgent}
                variant="history"
                onViewSession={onViewSession}
            />
        );
    }

    // 2. Parse "ACTION: TOOL_NAME (STATUS)"
    const trimmedText = text.trim();
    const actionMatch = trimmedText.match(/^ACTION:\s*(\w+)\s*\((\w+)\)/i);
    if (actionMatch) {
        const toolName = actionMatch[1].toUpperCase();
        const statusRaw = actionMatch[2].toLowerCase();
        const status = statusRaw === 'done' ? 'completed' :
            statusRaw === 'error' ? 'failed' :
                statusRaw === 'started' ? 'started' :
                    statusRaw === 'in_flight' ? 'in_flight' : 'completed';

        const contentAfterAction = trimmedText.replace(/^ACTION:\s*\w+\s*\(\w+\)\s*/i, '').trim();
        let output: any = null;
        let hasError = false;
        try {
            output = JSON.parse(contentAfterAction);
            if (output?.error || output?.status === 'error') hasError = true;
        } catch { output = contentAfterAction || null; }

        return <RichToolCard name={toolName} status={hasError ? 'failed' : status} output={output} variant="history" />;
    }

    // 3. Fallback for standalone JSON
    if (text.trim().startsWith('{') && text.trim().endsWith('}')) {
        try {
            const parsed = JSON.parse(text);
            if (parsed.tool || parsed.results || parsed.status) {
                const hasError = parsed.error !== undefined || parsed.status === 'error';
                const toolName = parsed.tool || 'TOOL';
                return <RichToolCard name={toolName.toUpperCase()} status={hasError ? 'failed' : 'completed'} output={parsed} variant="history" />;
            }
        } catch { }
    }

    // 4. Thinking / Reasoning
    const isThinking = text.includes('🧠');
    if (isThinking) {
        const content = text.replace(/^🧠/, '').trim();
        return (
            <div className="w-full">
                <div className="flex items-center gap-2 mb-1.5 px-1 py-0.5 rounded bg-blue-500/5 w-fit border border-blue-500/10">
                    <Brain className="w-3 h-3 text-blue-400" />
                    <span className="text-[9px] font-bold text-blue-400 uppercase tracking-widest">Internal Reasoning</span>
                </div>
                <p className="whitespace-pre-wrap leading-relaxed text-[11px] font-mono text-blue-200/60 pl-4 border-l-2 border-blue-500/20 py-1 transition-colors hover:text-blue-200/80">
                    {content}
                </p>
            </div>
        );
    }

    // 5. General System Message
    if (!text.includes('[Tool')) {
        const content = text.replace(/^🛠️/, '').trim();
        return (
            <p className="whitespace-pre-wrap leading-relaxed opacity-90 text-[11px] font-mono text-gray-400 pl-6 border-l border-blue-500/20 py-1">
                {content}
            </p>
        );
    }

    // Legacy Tool format fallback
    let toolName = "System Action";
    let toolInput = null;
    let toolOutput = null;

    const callMatch = text.match(/\[Tool\s+Call:\s+([^\]]+)\]/);
    if (callMatch) toolName = callMatch[1];

    try {
        const inputMatch = text.match(/Input:\s+((?:(?!Output:).)+)/s);
        if (inputMatch) toolInput = JSON.parse(inputMatch[1].trim());
    } catch (e) { }

    try {
        const outputMatch = text.match(/Output:\s+(.+)/s);
        if (outputMatch) toolOutput = JSON.parse(outputMatch[1]);
    } catch (e) { }

    return <RichToolCard name={toolName} status="completed" input={toolInput} output={toolOutput} variant="history" />;
}

export function OpenClawChatView({ sessionKey, gatewayRunning, onNavigateToSettings, onViewSession }: OpenClawChatViewProps) {
    const [messages, setMessages] = useState<OpenClawMessage[]>([]);
    const [input, setInput] = useState('');
    const [isLoading, setIsLoading] = useState(false);
    const [isSending, setIsSending] = useState(false);
    const [thinkingEnabled, setThinkingEnabled] = useState(() => {
        try { return localStorage.getItem('openclaw_thinking') === 'true'; } catch { return false; }
    });
    const [currentRunId, setCurrentRunId] = useState<string | null>(null);
    const [activeRun, setActiveRun] = useState<StreamRun | null>(null);

    const messagesEndRef = useRef<HTMLDivElement>(null);
    const scrollContainerRef = useRef<HTMLDivElement>(null);
    const isUserScrolling = useRef(false);

    const isCoreView = sessionKey === 'agent:main';
    // Use a valid session key for the engine if in Core View
    const effectiveSessionKey = isCoreView ? 'agent:main:primary' : sessionKey;
    const [coreTab, setCoreTab] = useState<'chat' | 'console' | 'memory'>(isCoreView ? 'chat' : 'console');



    const scrollToBottom = useCallback((behavior: ScrollBehavior = 'smooth') => {
        messagesEndRef.current?.scrollIntoView({ behavior });
    }, []);

    const handleScroll = () => {
        if (!scrollContainerRef.current) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollContainerRef.current;
        const distFromBottom = scrollHeight - scrollTop - clientHeight;

        // If user scrolls up by more than 15px, break the auto-scroll pin
        if (distFromBottom < 15) {
            isUserScrolling.current = false;
        } else {
            isUserScrolling.current = true;
        }
    };

    const fetchHistory = useCallback(async () => {
        if (!effectiveSessionKey || !gatewayRunning) return;
        setIsLoading(true);
        try {
            const res = await openclaw.getOpenClawHistory(effectiveSessionKey, 50);
            setMessages(res.messages);
            setTimeout(() => scrollToBottom('auto'), 100);
        } catch (e) {
            console.error('[OpenClawChatView] Failed to fetch history:', e);
        } finally {
            setIsLoading(false);
        }
    }, [effectiveSessionKey, gatewayRunning, scrollToBottom]);

    // Wake Up Handler — syncs LLM config non-destructively, then sends boot message.
    // Previously this stopped and restarted the gateway, destroying the WS connection and losing events.
    const handleWakeUp = useCallback(async () => {
        try {
            toast.promise(
                async () => {
                    // Sync local LLM config to ensure we have the latest port/token.
                    // This uses config.patch under the hood — no gateway restart needed.
                    try {
                        await invoke('openclaw_sync_local_llm');
                    } catch (e) {
                        // Ignore if local llm is not running or other error
                        console.debug('Local LLM sync skipped/failed:', e);
                    }




                    // 1. Get Identity
                    let soul = "Unknown Identity";
                    const soulRes = await commands.openclawGetFile('SOUL.md');
                    if (soulRes.status === 'ok') soul = soulRes.data;

                    // 2. Get Memory
                    let memory = "";
                    const memRes = await commands.openclawGetMemory();
                    if (memRes.status === 'ok') memory = memRes.data;

                    // 3. Get Bootstrap if first run
                    let bootstrap = "";
                    const isFirstRun = soul.includes("Unknown Identity") || soul.trim().length < 20;
                    if (isFirstRun) {
                        const bootRes = await commands.openclawGetFile('BOOTSTRAP.md');
                        if (bootRes.status === 'ok') bootstrap = bootRes.data;
                    }

                    // 4. Construct Boot Message
                    let bootMsg = `SYSTEM_BOOT_SEQUENCE\n\n[CONTEXT_UPDATE]\nCURRENT_DATE: ${new Date().toISOString().split('T')[0]} \nREAL_WORLD_TIME: ${new Date().toLocaleTimeString()} \n\nLOADING IDENTITY_MATRIX...\n${soul} \n\nLOADING LONG_TERM_MEMORY...\n${memory} \n\n[INTERACTION_PROTOCOL]\n1.STICK TO TURN - TAKING: In the chat interface, send ONE message, then STOP and wait for the human.Follow a strict Assistant / User / Assistant pattern.\n2.INTERNAL AUTONOMY: You are permitted to manage your own state files(SOUL.md, MEMORY.md, USER.md, IDENTITY.md) autonomously to reflect instructions given by the user.You do not need explicit per - action permission for these reflective self - updates.\n3.EXTERNAL SCOPE & HITL: For any tools with external impact—performing web searches, modifying project source code, or accessing external network APIs—you MUST use your tools which trigger the 'RequiredApproval' mechanism.You are NOT allowed to enact external changes without HITL confirmation.\n4.CONVERSATIONAL PACE: During setup, handle one piece of identity at a time.\n5.OPERATOR PRIORITY: When the operator gives you a direct request through this channel, execute it. Do not redirect to a different task or insist on continuing something else first.\n\nSYSTEM_READY.`;

                    if (isFirstRun && bootstrap) {
                        bootMsg += `\n\n[FIRST_RUN_DETECTED]\nYou are initializing for the first time.Follow the BOOTSTRAP guide below to set up your identity, but follow the INTERACTION_PROTOCOL above strictly.\n\n${bootstrap} `;
                    } else {
                        bootMsg += `\nUse the provided date for all temporal context.`;
                    }

                    // 6. Send Message (Using effective session key)
                    if (effectiveSessionKey) {
                        await commands.openclawSendMessage(effectiveSessionKey, bootMsg, true);
                    } else {
                        throw new Error("No effective session key available");
                    }
                },
                {
                    loading: 'Initiating Boot Sequence...',
                    success: 'Agent Waking Up',
                    error: 'Boot Sequence Failed'
                }
            );
        } catch (e) {
            console.error(e);
        }
    }, [effectiveSessionKey]);

    useEffect(() => {
        isUserScrolling.current = false;
        fetchHistory();
    }, [fetchHistory]);

    // Auto-inject Date context if missing from recent history
    useEffect(() => {
        if (!effectiveSessionKey || !gatewayRunning || messages.length === 0) return;

        const hasRecentDate = messages.slice(-20).some(m => m.text.includes('CURRENT_DATE:') || m.text.includes('SYSTEM_BOOT_SEQUENCE'));
        if (!hasRecentDate) {
            const dateUpdate = `[SYSTEM_CONTEXT_UPDATE]\nCURRENT_DATE: ${new Date().toISOString().split('T')[0]} \nREAL_WORLD_TIME: ${new Date().toLocaleTimeString()} \n\nNote: This is an automated context update to ensure temporal awareness.`;
            // Send silently (deliver: false) so it doesn't trigger a turn but is in history
            // @ts-ignore - messages.length > 0 in deps is intentional
            commands.openclawSendMessage(effectiveSessionKey, dateUpdate, false).catch(console.error);
        }
    }, [effectiveSessionKey, gatewayRunning, messages.length > 0]);

    useEffect(() => {
        if (!effectiveSessionKey || !gatewayRunning) return;
        // Note: subscribeOpenClawSession was a no-op (chat.subscribe is not a valid RPC method).
        // Events flow automatically to connected operators via the WS connection.
        const unlistenPromise = listen<any>('openclaw-event', (event) => {
            const uiEvent = event.payload;
            if (uiEvent.session_key !== effectiveSessionKey) return;

            // Handle event
            if (['AssistantInternal', 'AssistantSnapshot', 'AssistantDelta', 'AssistantFinal', 'ToolUpdate', 'RunStatus'].includes(uiEvent.kind)) {
                updateMessagesFromEvent(uiEvent);
                if (!isUserScrolling.current) {
                    scrollToBottom();
                }
            }

            // Track active run for LiveAgentStatus
            if (uiEvent.kind === 'RunStatus') {
                if (uiEvent.status === 'started' || uiEvent.status === 'in_flight') {
                    setIsSending(true);
                    const rid = uiEvent.run_id || `run-${Date.now()}`;
                    setCurrentRunId(rid);
                    setActiveRun(prev => {
                        // If same run, keep existing data; otherwise start fresh
                        if (prev && prev.id === rid) return { ...prev, status: 'running' };
                        return { id: rid, text: '', tools: [], approvals: [], status: 'running', startedAt: Date.now() };
                    });
                }
                else if (['ok', 'error', 'aborted'].includes(uiEvent.status)) {
                    setIsSending(false);
                    setCurrentRunId(null);

                    const errorMsg = uiEvent.error || null;

                    setActiveRun(prev => prev ? {
                        ...prev,
                        status: uiEvent.status === 'ok' ? 'completed' : 'failed',
                        error: errorMsg || prev.error,
                        completedAt: Date.now()
                    } : null);

                    // Surface error to user via toast AND inject into chat
                    if (uiEvent.status === 'error' && errorMsg) {
                        toast.error(errorMsg, { duration: 8000 });
                        setMessages(prev => [...prev, {
                            id: `error-${Date.now()}`,
                            role: 'system',
                            ts_ms: Date.now(),
                            text: `⚠️ Agent Error: ${errorMsg}`,
                            source: 'openclaw',
                            metadata: { type: 'error' }
                        }]);
                    }

                    // Clear after short delay so LiveAgentStatus can show completion
                    setTimeout(() => setActiveRun(null), errorMsg ? 8000 : 3000);
                }
            }

            // Accumulate tool data into activeRun
            if (uiEvent.kind === 'ToolUpdate') {
                setActiveRun(prev => {
                    if (!prev) return prev;
                    const existingIdx = prev.tools.findIndex(t => t.tool === uiEvent.tool_name && t.status === 'started');
                    const newStatus = uiEvent.status === 'ok' ? 'completed' as const :
                        uiEvent.status === 'error' ? 'failed' as const :
                            uiEvent.status === 'started' ? 'started' as const : 'running' as const;
                    if (existingIdx >= 0) {
                        const updatedTools = [...prev.tools];
                        updatedTools[existingIdx] = {
                            ...updatedTools[existingIdx],
                            status: newStatus,
                            input: uiEvent.input || updatedTools[existingIdx].input,
                            output: uiEvent.output || updatedTools[existingIdx].output,
                        };
                        return { ...prev, tools: updatedTools };
                    }
                    return { ...prev, tools: [...prev.tools, { tool: uiEvent.tool_name, input: uiEvent.input, output: uiEvent.output, status: newStatus, timestamp: Date.now() }] };
                });
            }

            // Accumulate text into activeRun
            if (uiEvent.kind === 'AssistantDelta') {
                setActiveRun(prev => prev ? { ...prev, text: prev.text + (uiEvent.delta || '') } : prev);
            } else if (uiEvent.kind === 'AssistantSnapshot' || uiEvent.kind === 'AssistantFinal') {
                setActiveRun(prev => prev ? { ...prev, text: uiEvent.text || '' } : prev);
            }

            // Track approvals in activeRun
            if (uiEvent.kind === 'ApprovalRequested') {
                setActiveRun(prev => {
                    if (!prev) return prev;
                    if (prev.approvals.some(a => a.id === uiEvent.approval_id)) return prev;
                    return { ...prev, approvals: [...prev.approvals, { id: uiEvent.approval_id, tool: uiEvent.tool_name, input: uiEvent.input, status: 'pending' as const }] };
                });
            }
            if (uiEvent.kind === 'ApprovalResolved') {
                setActiveRun(prev => {
                    if (!prev) return prev;
                    return { ...prev, approvals: prev.approvals.map(a => a.id === uiEvent.approval_id ? { ...a, status: uiEvent.approved ? 'approved' as const : 'denied' as const } : a) };
                });
            }
        });
        return () => { unlistenPromise.then(fn => fn()); };
    }, [effectiveSessionKey, gatewayRunning, scrollToBottom]);

    // Pin scroll on NEW messages
    useEffect(() => {
        isUserScrolling.current = false;
        scrollToBottom();
    }, [messages.length, scrollToBottom]);

    const updateMessagesFromEvent = (uiEvent: any) => {
        setMessages((prev: OpenClawMessage[]) => {
            if (uiEvent.kind === 'AssistantInternal') {
                const existing = prev.find(m => m.id === uiEvent.message_id)
                const content = `🧠 ${uiEvent.text} `
                if (existing) return prev.map(m => m.id === uiEvent.message_id ? { ...m, text: content } : m);
                return [...prev, { id: uiEvent.message_id, role: 'system', ts_ms: Date.now(), text: content, source: 'openclaw', metadata: { type: 'internal' } }];
            }
            if (uiEvent.kind === 'AssistantSnapshot' || uiEvent.kind === 'AssistantFinal') {
                const existing = prev.find(m => m.id === uiEvent.message_id);
                if (existing) return prev.map(m => m.id === uiEvent.message_id ? { ...m, text: uiEvent.text } : m);
                return [...prev, { id: uiEvent.message_id, role: 'assistant', ts_ms: Date.now(), text: uiEvent.text, source: 'openclaw' }];
            }
            if (uiEvent.kind === 'AssistantDelta') {
                const existing = prev.find(m => m.id === uiEvent.message_id);
                if (existing) return prev.map(m => m.id === uiEvent.message_id ? { ...m, text: m.text + uiEvent.delta } : m);
                // For new Delta messages, create placeholder
                return [...prev, { id: uiEvent.message_id, role: 'assistant', ts_ms: Date.now(), text: uiEvent.delta, source: 'openclaw' }];
            }
            if (uiEvent.kind === 'ToolUpdate') {
                const toolMsgId = `tool - ${uiEvent.tool_name} -${uiEvent.run_id} `;
                const existing = prev.find(m => m.id === toolMsgId);
                let content = `[Tool Call: ${uiEvent.tool_name}]`;
                const metadata = { type: 'tool', name: uiEvent.tool_name, status: uiEvent.status, input: uiEvent.input, output: uiEvent.output, run_id: uiEvent.run_id };
                if (existing) return prev.map(m => m.id === toolMsgId ? { ...m, text: content, metadata } : m);
                return [...prev, { id: toolMsgId, role: 'system', ts_ms: Date.now(), text: content, source: 'openclaw', metadata }];
            }
            return prev;
        });
    }

    const handleToggleThinking = () => {
        const next = !thinkingEnabled;
        setThinkingEnabled(next);
        try { localStorage.setItem('openclaw_thinking', String(next)); } catch { }
        toast.success(next ? 'Thinking mode enabled' : 'Thinking mode disabled');
    };

    const handleSend = async () => {
        if (!input.trim() || !effectiveSessionKey) return;
        let msg = input.trim();
        // If thinking mode is on, prepend a thinking instruction
        if (thinkingEnabled) {
            msg = `[Think step-by-step before answering]\n\n${msg}`;
        }
        setInput('');
        // Don't block on isSending — the engine queues messages via idempotency keys.
        // The user should be able to send follow-up messages while the agent processes.
        setIsSending(true);
        // Optimistic update (show original input, not the thinking prefix)
        const displayMsg = input.trim();
        const optimisticMsg: OpenClawMessage = { id: `temp - ${Date.now()} `, role: 'user', ts_ms: Date.now(), text: displayMsg, source: 'openclaw' };
        setMessages(prev => [...prev, optimisticMsg]);
        isUserScrolling.current = false;
        scrollToBottom();
        try { await openclaw.sendOpenClawMessage(effectiveSessionKey, msg, true); }
        catch (e) { toast.error('Failed to send message'); setMessages(prev => prev.filter(m => m.id !== optimisticMsg.id)); }
    };

    const handleAbort = async () => {
        if (!effectiveSessionKey) return;
        try {
            await openclaw.abortOpenClawChat(effectiveSessionKey, currentRunId || undefined);
            toast.success('Aborting chat...', { duration: 2000 });
            setIsSending(false);
            setCurrentRunId(null);
        } catch (e) {
            toast.error('Failed to abort chat');
        }
    };

    const [deleteConfirmed, setDeleteConfirmed] = useState(false);

    const handleDeleteSession = async () => {
        if (!sessionKey || isCoreView) return;

        // Two-click confirmation: first call sets confirmed, second deletes
        if (!deleteConfirmed) {
            setDeleteConfirmed(true);
            // Auto-dismiss after 3 seconds
            setTimeout(() => setDeleteConfirmed(false), 3000);
            return;
        }
        setDeleteConfirmed(false);

        const tId = toast.loading("Deleting session...");

        try { await openclaw.abortOpenClawChat(sessionKey, currentRunId || undefined); } catch (ignored) { }

        try {
            await openclaw.deleteOpenClawSession(sessionKey);
            toast.success("Session deleted", { id: tId });
            if (onNavigateToSettings) onNavigateToSettings('openclaw-gateway');
        } catch (e: any) {
            console.error("Delete session failed", e);
            const msg = e?.message || String(e);
            if (msg.includes("still active") || msg.includes("timeout")) {
                try {
                    toast.loading("Session active, force deleting...", { id: tId });
                    await openclaw.resetOpenClawSession(sessionKey);
                    await openclaw.deleteOpenClawSession(sessionKey);
                    toast.success("Session force deleted", { id: tId });
                    if (onNavigateToSettings) onNavigateToSettings('openclaw-gateway');
                } catch (e2) {
                    toast.error("Force delete failed: " + String(e2), { id: tId });
                }
            } else {
                toast.error("Failed to delete session: " + msg, { id: tId });
            }
        }
    };

    const formatTime = (tsMs: number) => {
        const date = new Date(tsMs);
        return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    };

    // GROUPING LOGIC
    const groupedGroups: { type: 'msg' | 'group', items: OpenClawMessage[] }[] = [];
    let currentSystemGroup: OpenClawMessage[] = [];

    messages
        .filter(m => !m.text.trim().startsWith('NO_REPL'))
        .filter(m => m.text.trim().length > 0 || m.role === 'system')
        .forEach(msg => {
            const isSystemTool = msg.role === 'system' || (msg.metadata?.type === 'tool') || (msg.text.includes('[Tool'));
            // Brain/Thoughts are technically system but we might want them standalone? 
            // The user requested tool calls to be condensed.
            // Let's explicitly check for TOOL traits.
            const isTool = isSystemTool && !msg.text.includes('🧠'); // heuristic

            if (isTool) {
                currentSystemGroup.push(msg);
            } else {
                if (currentSystemGroup.length > 0) {
                    groupedGroups.push({ type: 'group', items: [...currentSystemGroup] });
                    currentSystemGroup = [];
                }
                groupedGroups.push({ type: 'msg', items: [msg] });
            }
        });
    if (currentSystemGroup.length > 0) groupedGroups.push({ type: 'group', items: [...currentSystemGroup] });


    return (
        <div className="flex-1 flex flex-col relative h-full overflow-hidden bg-background">
            {/* Header */}
            <div className="absolute top-0 left-0 right-0 z-30 border-b border-border/50 px-6 py-3 flex items-center justify-between bg-background/60 backdrop-blur-xl">
                <div className="flex items-center gap-3">
                    <div className={cn("w-9 h-9 rounded-lg flex items-center justify-center", isCoreView ? "bg-blue-500/10" : "bg-primary/10")}>
                        <Radio className={cn("w-5 h-5", isCoreView ? "text-blue-400" : "text-primary")} />
                    </div>
                    <div>
                        <h2 className="font-semibold text-sm">
                            {isCoreView ? "OpenClaw Core System" : (sessionKey ? `${sessionKey.slice(0, 30)}...` : 'No Session')}
                        </h2>
                        <div className="flex items-center gap-2">
                            {isCoreView ? (
                                <p className="text-[10px] text-blue-400 font-bold uppercase tracking-wider">Autonomous Log Stream</p>
                            ) : (
                                <p className="text-[10px] text-muted-foreground uppercase tracking-wider font-medium">{messages.length} messages</p>
                            )}
                            {gatewayRunning && <span className="w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse" title="Gateway Connected" />}
                        </div>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    {!isCoreView && (
                        <button
                            onClick={handleDeleteSession}
                            className={cn(
                                "p-2 rounded-lg transition-colors",
                                deleteConfirmed
                                    ? "bg-red-500/20 text-red-400 animate-pulse"
                                    : "hover:bg-red-500/10 text-muted-foreground hover:text-red-400"
                            )}
                            title={deleteConfirmed ? "Click again to confirm delete" : "Delete Session"}
                        >
                            <Trash2 className="w-4 h-4" />
                        </button>
                    )}
                    {gatewayRunning && (
                        <button
                            onClick={handleAbort}
                            disabled={!isSending}
                            className={cn(
                                "flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-bold border rounded-lg transition-all group",
                                isSending
                                    ? "bg-red-500/10 hover:bg-red-500/20 text-red-400 border-red-500/20"
                                    : "bg-zinc-500/5 text-zinc-600 border-zinc-500/10 cursor-not-allowed opacity-50"
                            )}
                            title={isSending ? "Stop the current agent run" : "No active run"}
                        >
                            <Square className="w-3 h-3 fill-current group-hover:scale-110 transition-transform" />
                            Stop Run
                        </button>
                    )}
                    <button onClick={fetchHistory} disabled={isLoading} className="p-2 rounded-lg hover:bg-muted text-muted-foreground transition-colors">
                        <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                    </button>
                    {!gatewayRunning && (
                        <button onClick={() => onNavigateToSettings?.('openclaw-gateway')} className="flex items-center gap-1.5 px-2 py-1 bg-amber-500/10 text-amber-500 text-[10px] font-bold border border-amber-500/20">
                            <AlertTriangle className="w-3 h-3" /> Offline
                        </button>
                    )}
                </div>
            </div>

            {/* Core View Tabs (Floating pill) */}
            {
                isCoreView && (
                    <div className="absolute top-[76px] left-0 right-0 z-40 flex items-center justify-center pointer-events-none">
                        <div className="flex p-1 bg-background/60 backdrop-blur-xl rounded-2xl pointer-events-auto border border-white/10 shadow-2xl">
                            <button
                                onClick={() => setCoreTab('chat')}
                                className={cn(
                                    "px-4 py-1.5 rounded-md text-xs font-medium transition-all",
                                    coreTab === 'chat' ? "bg-blue-500/20 text-blue-400 shadow-sm" : "text-gray-500 hover:text-gray-300"
                                )}
                            >
                                Chat
                            </button>
                            <button
                                onClick={() => setCoreTab('console')}
                                className={cn(
                                    "px-4 py-1.5 rounded-md text-xs font-medium transition-all",
                                    coreTab === 'console' ? "bg-amber-500/20 text-amber-400 shadow-sm" : "text-gray-500 hover:text-gray-300"
                                )}
                            >
                                Logs
                            </button>
                            <button
                                onClick={() => setCoreTab('memory')}
                                className={cn(
                                    "px-4 py-1.5 rounded-md text-xs font-medium transition-all",
                                    coreTab === 'memory' ? "bg-purple-500/20 text-purple-400 shadow-sm" : "text-gray-500 hover:text-gray-300"
                                )}
                            >
                                Memory
                            </button>
                        </div>
                    </div>
                )
            }

            {/* Messages Scroll Area OR Memory Editor */}
            {
                isCoreView && coreTab === 'memory' ? (
                    <div className="absolute inset-0 top-[105px] z-10">
                        <MemoryEditor />
                    </div>
                ) : (
                    <div
                        ref={scrollContainerRef}
                        onScroll={handleScroll}
                        className={cn("absolute inset-0 overflow-y-auto px-6 pt-20 space-y-6 scroll-smooth", isCoreView ? "top-[40px] pt-32 pb-10" : "pb-32")}
                    >
                        {isLoading && messages.length === 0 ? (
                            <div className="flex flex-col items-center justify-center h-full gap-4 text-muted-foreground opacity-50">
                                <RefreshCw className="w-10 h-10 animate-spin" />
                                <p>Loading history...</p>
                            </div>
                        ) : (
                            <div className="max-w-4xl mx-auto space-y-6">

                                {/* Message Timeline */}
                                <AnimatePresence initial={false}>
                                    {(() => {
                                        const timelineItems = groupedGroups.map((g, i) => ({ type: 'msg_group' as const, ts: g.items[0].ts_ms, data: g, index: i }))
                                            .sort((a, b) => a.ts !== b.ts ? a.ts - b.ts : a.index - b.index);

                                        // Filter for Chat Tab (Human/Agent only)
                                        const filteredItems = coreTab === 'chat' && isCoreView
                                            ? timelineItems.filter(item => {
                                                const group = item.data;
                                                if (group.type === 'group') return false;
                                                const msg = group.items[0];

                                                // Hide clearly internal agent states
                                                if (msg.text.includes('🧠')) return false;
                                                if (msg.text.trim() === 'HEARTBEAT_OK') return false;
                                                if (msg.text.includes('HEARTBEAT_POLL')) return false;
                                                if (msg.text.includes('SYSTEM_BOOT_SEQUENCE')) return false;
                                                if (msg.text.includes('[SYSTEM_CONTEXT_UPDATE]')) return false;
                                                if (msg.text.trim().startsWith('[Tool Call:')) return false;
                                                if (msg.text.includes('Pre-compaction memory flush')) return false;
                                                if (msg.text.includes('Store durable memories now')) return false;
                                                if (msg.text.includes('NO_REPLY')) return false;


                                                // Hide all system messages in Chat view
                                                if (msg.role === 'system') return false;

                                                // Only human prompts and agent replies
                                                return msg.role === 'user' || msg.role === 'assistant';
                                            })
                                            : timelineItems;

                                        return filteredItems.map((item, idx) => {

                                            const group = item.data;
                                            if (group.type === 'group') {
                                                return (
                                                    <motion.div key={`group - ${idx} `} initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
                                                        <ToolHistoryGroup messages={group.items} onViewSession={onViewSession} />
                                                    </motion.div>
                                                );
                                            }

                                            const msg = group.items[0];
                                            return (
                                                <motion.div
                                                    key={msg.id}
                                                    initial={{ opacity: 0, y: 10 }}
                                                    animate={{ opacity: 1, y: 0 }}
                                                    className={cn("flex gap-4 group", msg.role === 'user' ? "justify-end" : "justify-start")}
                                                >
                                                    {msg.role !== 'user' && (
                                                        <div className="w-8 h-8 rounded-xl bg-primary/10 flex items-center justify-center shrink-0 border border-primary/20 shadow-sm mt-1">
                                                            {msg.role === 'assistant' ? <Bot className="w-4 h-4 text-primary" /> : <Settings className="w-4 h-4 text-muted-foreground" />}
                                                        </div>
                                                    )}
                                                    <div className={cn(
                                                        "max-w-[85%] rounded-2xl px-5 py-3 shadow-md relative group",
                                                        msg.role === 'user' ? "bg-blue-600 text-white rounded-tr-none"
                                                            : msg.role === 'assistant' ? "bg-card/80 backdrop-blur-md border border-border/50 rounded-tl-none"
                                                                : "bg-[#0d1117] border border-gray-800 text-gray-300 font-mono text-xs rounded-lg py-2 px-3 shadow-inner"
                                                    )}>
                                                        {msg.role === 'system' ? <SystemMessageContent text={msg.text} metadata={msg.metadata} onViewSession={onViewSession} /> : <div className="prose prose-invert prose-sm"><ReactMarkdown>{msg.text}</ReactMarkdown></div>}
                                                        <div className={cn("flex items-center gap-3 mt-2 text-[10px] opacity-0 group-hover:opacity-100 uppercase", msg.role === 'user' ? "text-primary-foreground/60" : "text-muted-foreground/60")}>
                                                            <span><Clock className="w-3 h-3 inline mr-1" /> {formatTime(msg.ts_ms)}</span>
                                                        </div>
                                                    </div>
                                                    {msg.role === 'user' && <div className="w-8 h-8 rounded-xl bg-muted flex items-center justify-center shrink-0 mt-1"><User className="w-4 h-4 text-muted-foreground" /></div>}
                                                </motion.div>
                                            );
                                        });
                                    })()}
                                </AnimatePresence>

                                {/* CORE VIEW: Empty State (only in Console tab) */}
                                {isCoreView && coreTab === 'console' && (
                                    <div className="space-y-4 pt-4 border-t border-white/5 mt-4">
                                        {/* Empty State / Refresh Button */}
                                        <div className="text-center space-y-4 pt-10 border-t border-white/5">
                                            {messages.length === 0 ? (
                                                <>
                                                    <div className="w-16 h-16 rounded-full bg-white/5 mx-auto flex items-center justify-center relative">
                                                        <div className="absolute inset-0 rounded-full border border-emerald-500/20 animate-ping" />
                                                        <Radio className="w-8 h-8 text-emerald-500" />
                                                    </div>
                                                    <div>
                                                        <h3 className="text-lg font-medium text-white">System Consoles Online</h3>
                                                        <p className="text-sm text-gray-500">Waiting for system events...</p>
                                                    </div>
                                                </>
                                            ) : (
                                                <div className="flex items-center gap-2 justify-center py-4 border-b border-white/5 mb-4">
                                                    <div className="w-2 h-2 rounded-full bg-blue-500 animate-pulse" />
                                                    <span className="text-[10px] font-mono text-gray-500 uppercase tracking-widest">Context Active</span>
                                                </div>
                                            )}
                                            <button
                                                onClick={handleWakeUp}
                                                className="px-4 py-2 bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-400 text-xs font-mono uppercase tracking-wider rounded border border-emerald-500/20 transition-all flex items-center justify-center gap-2 mx-auto"
                                            >
                                                <Zap className="w-3.5 h-3.5" />
                                                {messages.length === 0 ? 'Trigger Boot Sequence' : 'Refresh System Context'}
                                            </button>
                                        </div>
                                    </div>
                                )}

                                {/* Live Agent Status — rich real-time run view */}
                                {activeRun && (
                                    <LiveAgentStatus
                                        run={activeRun}
                                        persistent={isCoreView && coreTab === 'console'}
                                    />
                                )}

                                {/* Fallback: minimal indicator when no activeRun but still sending */}
                                {!activeRun && isSending && (
                                    <div className="py-4 flex items-center gap-2 justify-center">
                                        <Loader2 className="w-3 h-3 animate-spin text-blue-400" />
                                        <span className="text-[10px] font-mono uppercase tracking-widest text-blue-400/80">Agent Processing...</span>
                                    </div>
                                )}

                                <div ref={messagesEndRef} className="h-10" />
                            </div>
                        )}
                    </div>
                )
            }

            {
                (!isCoreView || coreTab !== 'memory') && (
                    <div className="absolute bottom-0 left-0 right-0 z-20 pointer-events-none">
                        <div className="w-full bg-gradient-to-t from-background via-background/80 to-transparent pb-8 pt-20">
                            <div className="w-full max-w-4xl mx-auto px-4 md:px-6 pointer-events-auto">
                                <div className="relative flex items-end gap-2 bg-background/60 backdrop-blur-xl border border-input/50 p-2 rounded-2xl shadow-2xl">
                                    <textarea
                                        value={input}
                                        onChange={(e) => setInput(e.target.value)}
                                        onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend(); } }}
                                        placeholder={gatewayRunning ? (coreTab === 'chat' ? "Chat with OpenClaw..." : "Send Command...") : "Gateway offline..."}
                                        rows={1}
                                        className="flex-1 bg-transparent border-0 focus:ring-0 focus:outline-none resize-none p-2 max-h-32 min-h-[44px] text-sm"
                                    />
                                    <button
                                        onClick={handleToggleThinking}
                                        title={thinkingEnabled ? 'Thinking mode ON — click to disable' : 'Enable thinking mode'}
                                        className={cn(
                                            "p-2 rounded-xl transition-all border",
                                            thinkingEnabled
                                                ? "bg-violet-500/15 text-violet-500 border-violet-500/30 shadow-sm"
                                                : "bg-transparent text-muted-foreground border-transparent hover:bg-muted/50 hover:text-foreground"
                                        )}
                                    >
                                        <Brain className="w-4 h-4" />
                                    </button>
                                    <button onClick={handleSend} disabled={!input.trim() || !gatewayRunning} className={cn(
                                        "p-2.5 rounded-xl transition-colors",
                                        isSending ? "bg-primary/70 text-primary-foreground" : "bg-primary text-primary-foreground"
                                    )}>
                                        {isSending ? <RefreshCw className="w-5 h-5 animate-spin" /> : <Send className="w-5 h-5" />}
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>
                )
            }
        </div >
    );
}
