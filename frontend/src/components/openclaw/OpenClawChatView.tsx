import { invoke } from '@tauri-apps/api/core';

import { useState, useEffect, useCallback, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Send, Radio, RefreshCw, AlertTriangle, Clock, User, Bot, Settings, ChevronRight, ChevronDown, Brain, Terminal, Loader2, CheckCircle2, XCircle, Layers, Zap, ExternalLink, Trash2, Download, Sliders, FileDown, PanelRight, Copy, Check } from 'lucide-react';
import { commands } from '../../lib/bindings';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import * as openclaw from '../../lib/openclaw';
import { OpenClawMessage } from '../../lib/openclaw';
import { listen } from '@tauri-apps/api/event';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { StreamRun } from '../../hooks/use-openclaw-stream';
import { LiveAgentStatus } from './LiveAgentStatus';
import { MemoryEditor } from './MemoryEditor';
import SubAgentPanel, { useSubAgentCount } from './SubAgentPanel';
import AutomationCard from './AutomationCard';
import { Square } from 'lucide-react';

interface OpenClawChatViewProps {
    sessionKey: string | null;
    gatewayRunning: boolean;
    /** True on first run — agent should lead the identity bootstrap ritual */
    bootstrapNeeded?: boolean;
    /** Called when bootstrap ritual is detected as complete via memory_delete */
    onBootstrapComplete?: () => void;
    /** Called on factory reset so parent can re-check bootstrap state from backend */
    onFactoryReset?: () => void;
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

function RichToolCard({ name, status, input: rawInput, output: rawOutput, isSubAgent, variant = 'live', onViewSession }: RichToolCardProps) {
    const [isExpanded, setIsExpanded] = useState(false);

    // Filter out null/undefined/"null"/"Null" to prevent showing empty data
    const input = (rawInput === null || rawInput === undefined || rawInput === 'null' || rawInput === 'Null') ? undefined : rawInput;
    const output = (rawOutput === null || rawOutput === undefined || rawOutput === 'null' || rawOutput === 'Null') ? undefined : rawOutput;

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
                            <div className="bg-black/30 rounded p-2 border border-border/40">
                                <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-1">Input</div>
                                <pre className="text-[10px] font-mono text-gray-300 overflow-x-auto whitespace-pre-wrap">
                                    {typeof input === 'string' ? input : JSON.stringify(input, null, 2)}
                                </pre>
                            </div>
                        )}
                        {output && (
                            <div className="bg-black/30 rounded p-2 border border-border/40">
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

// Shared markdown components for OpenClaw chat — handles link clicks
// by opening URLs in the system browser (Safari/Chrome) instead of
// navigating the Tauri webview.
const markdownComponents = {
    a: ({ node, href, children, ...props }: any) => {
        const isExternalUrl = href && (href.startsWith('http://') || href.startsWith('https://'));
        return (
            <a
                {...props}
                href={href}
                className="text-primary hover:underline cursor-pointer"
                onClick={(e: React.MouseEvent) => {
                    if (isExternalUrl && href) {
                        e.preventDefault();
                        import('@tauri-apps/plugin-opener').then(({ openUrl }) => {
                            openUrl(href);
                        }).catch(() => {
                            window.open(href, '_blank', 'noopener,noreferrer');
                        });
                    }
                }}
            >
                {children}
            </a>
        );
    },
};

// Parse [TOOL_CALLS] in assistant messages and render nicely
function AssistantMessageContent({ text }: { text: string }) {
    // Match patterns like: [TOOL_CALLS]tool_name[ARGS]{"key": "value"}
    // Use greedy match to end-of-line for the JSON args (handles nested objects)
    const toolCallRegex = /\[TOOL_CALLS\](\w+)\[ARGS\](\{.*?\})(?:\s|$)/gm;
    const hasToolCalls = toolCallRegex.test(text);

    if (!hasToolCalls) {
        return <div className="prose prose-invert prose-sm"><ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{text}</ReactMarkdown></div>;
    }

    // Reset regex after test
    toolCallRegex.lastIndex = 0;

    // Split into text parts and tool call parts
    const parts: { type: 'text' | 'tool'; content?: string; toolName?: string; toolInput?: any }[] = [];
    let lastIndex = 0;
    let match;

    while ((match = toolCallRegex.exec(text)) !== null) {
        // Add preceding text if any
        if (match.index > lastIndex) {
            const preceding = text.slice(lastIndex, match.index).trim();
            if (preceding) {
                parts.push({ type: 'text', content: preceding });
            }
        }

        // Parse tool call
        let parsedInput: any = undefined;
        try {
            parsedInput = JSON.parse(match[2]);
        } catch {
            parsedInput = match[2];
        }

        parts.push({
            type: 'tool',
            toolName: match[1],
            toolInput: parsedInput,
        });

        lastIndex = match.index + match[0].length;
    }

    // Add trailing text if any
    if (lastIndex < text.length) {
        const trailing = text.slice(lastIndex).trim();
        if (trailing) {
            parts.push({ type: 'text', content: trailing });
        }
    }

    return (
        <div className="space-y-2">
            {parts.map((part, i) => {
                if (part.type === 'text' && part.content) {
                    return <div key={i} className="prose prose-invert prose-sm"><ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{part.content}</ReactMarkdown></div>;
                }
                if (part.type === 'tool' && part.toolName) {
                    return (
                        <RichToolCard
                            key={i}
                            name={part.toolName}
                            status="completed"
                            input={part.toolInput}
                            variant="history"
                        />
                    );
                }
                return null;
            })}
        </div>
    );
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
                    expanded ? "bg-white/5 border-border/40" : "bg-transparent border-transparent hover:bg-white/5",
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

// ── Copy-to-clipboard for chat messages ─────────────────────────────────

function CopyMessageButton({ text }: { text: string }) {
    const [copied, setCopied] = useState(false);
    return (
        <button
            onClick={async (e) => {
                e.stopPropagation();
                try {
                    await navigator.clipboard.writeText(text);
                    setCopied(true);
                    setTimeout(() => setCopied(false), 2000);
                } catch { /* noop */ }
            }}
            title="Copy message"
            className={cn(
                'inline-flex items-center gap-0.5 transition-all duration-200',
                copied
                    ? 'text-emerald-400 scale-110'
                    : 'hover:text-foreground/70 hover:scale-105'
            )}
        >
            {copied
                ? <><Check className="w-3 h-3" /> <span className="normal-case">copied</span></>
                : <Copy className="w-3 h-3" />
            }
        </button>
    );
}

function SystemMessageContent({ text, metadata, onViewSession }: { text: string; metadata?: any; onViewSession?: (sessionKey: string) => void }) {
    // 1. File Created Card
    if (metadata?.type === 'file_created') {
        const kb = metadata.bytes < 1024
            ? `${metadata.bytes} B`
            : `${(metadata.bytes / 1024).toFixed(1)} KB`;
        return (
            <div className="flex items-start gap-3 p-3 rounded-xl border border-emerald-500/20 bg-emerald-500/5 w-full max-w-md">
                <div className="p-2 bg-emerald-500/10 rounded-lg shrink-0">
                    <svg className="w-4 h-4 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                    </svg>
                </div>
                <div className="flex-1 min-w-0">
                    <p className="text-[10px] font-bold text-emerald-400 uppercase tracking-widest mb-0.5">File Created</p>
                    <p className="text-xs font-mono text-foreground/80 truncate">{metadata.relative_path || metadata.absolute_path?.split('/').pop()}</p>
                    <p className="text-[10px] text-muted-foreground mt-0.5">{kb}</p>
                </div>
                <button
                    onClick={() => {
                        (window as any).__tauri__?.invoke('openclaw_reveal_file', { path: metadata.absolute_path }).catch(() => { });
                    }}
                    title="Reveal in Finder"
                    className="p-1.5 rounded-lg bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-400 transition-all border border-emerald-500/20 shrink-0"
                >
                    <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                        <path strokeLinecap="round" strokeLinejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                    </svg>
                </button>
            </div>
        );
    }

    // 2. Rich Tool Card Support (Backend Metadata)
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

export function OpenClawChatView({ sessionKey, gatewayRunning, bootstrapNeeded = false, onBootstrapComplete = () => { }, onFactoryReset, onNavigateToSettings, onViewSession }: OpenClawChatViewProps) {
    const [messages, setMessages] = useState<OpenClawMessage[]>([]);
    const [input, setInput] = useState('');
    const [isLoading, setIsLoading] = useState(false);
    const [isSending, setIsSending] = useState(false);
    const [thinkingEnabled, setThinkingEnabled] = useState(false);
    const [thinkingBudget, setThinkingBudget] = useState<number>(8192);
    const [showThinkingSlider, setShowThinkingSlider] = useState(false);
    const [currentRunId, setCurrentRunId] = useState<string | null>(null);
    const [activeRun, setActiveRun] = useState<StreamRun | null>(null);
    const [subAgentPanelOpen, setSubAgentPanelOpen] = useState(false);
    const [subAgentPanelDismissed, setSubAgentPanelDismissed] = useState(false);
    const subAgentCount = useSubAgentCount(sessionKey || '');

    // Auto-open the panel when first sub-agent appears
    useEffect(() => {
        if (subAgentCount > 0 && !subAgentPanelDismissed) {
            setSubAgentPanelOpen(true);
        }
    }, [subAgentCount, subAgentPanelDismissed]);

    const messagesEndRef = useRef<HTMLDivElement>(null);
    const scrollContainerRef = useRef<HTMLDivElement>(null);
    const isUserScrolling = useRef(false);

    // Inference speed tracking for OpenClaw
    const ocStreamStartRef = useRef<number | null>(null);
    const ocCharsReceivedRef = useRef<number>(0);
    const ocActiveMessageIdRef = useRef<string | null>(null);

    const isCoreView = sessionKey === 'agent:main';
    // Use a valid session key for the engine if in Core View
    const effectiveSessionKey = isCoreView ? 'agent:main' : sessionKey;
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
        // Don't gate on gatewayRunning — the DB has history even if the
        // polling interval hasn't confirmed the gateway yet. This prevents
        // the empty-chat-on-remount race condition.
        if (!effectiveSessionKey) return;
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
    }, [effectiveSessionKey, scrollToBottom]);

    // Wake Up Handler — sends a lightweight context refresh to IronClaw.
    //
    // IronClaw manages its own identity, memory, and workspace files internally
    // via its agent system and tools. We do NOT read SOUL.md/MEMORY.md/BOOTSTRAP.md
    // from the frontend — those are IronClaw-internal concerns.
    //
    // This handler just:
    //  1. Syncs the local LLM config (port/token)
    //  2. Sends a timestamped context message so the agent knows the current time
    const handleWakeUp = useCallback(async (isBootstrap?: boolean) => {
        if (!effectiveSessionKey || !gatewayRunning) {
            toast.error('Gateway is not running. Start it from Gateway Settings.');
            return;
        }
        try {
            setIsSending(true);
            const tempRunId = `wake-${Date.now()}`;
            setCurrentRunId(tempRunId);
            setActiveRun({
                id: tempRunId,
                text: '',
                tools: [],
                approvals: [],
                status: 'running',
                startedAt: Date.now(),
            });

            // Sync local LLM config first
            try { await invoke('openclaw_sync_local_llm'); } catch { /* non-fatal */ }

            const now = new Date();
            const dateStr = now.toISOString().split('T')[0];
            const timeStr = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

            let contextMsg: string;

            if (isBootstrap) {
                // Bootstrap mode: BOOTSTRAP.md IS the entire system prompt.
                // The agent's ONLY instruction source right now is that file.
                // Send the absolute minimum needed — just date/time context —
                // and let BOOTSTRAP.md drive the conversation.
                // Do NOT send a "Begin." or additional instructions here;
                // they fight with BOOTSTRAP.md and produce hybrid responses.
                contextMsg = [
                    `[BOOT_SEQUENCE: BOOTSTRAP]`,
                    `DATE: ${dateStr}  TIME: ${timeStr}`,
                ].join('\n');
            } else {
                // Normal session start: warm, brief, in-character greeting.
                // The agent reads SOUL.md / IDENTITY.md / MEMORY.md from its system prompt
                // and knows who it is — it should just say hello.
                contextMsg = [
                    `[BOOT_SEQUENCE: SESSION_START]`,
                    `DATE: ${dateStr}  TIME: ${timeStr}`,
                    ``,
                    `You are coming online for a new session. Greet your user briefly and warmly,`,
                    `in your established voice. Read your daily log and MEMORY.md for session context.`,
                    `If there's anything time-sensitive or worth noting, mention it.`,
                    `Keep it short — let the conversation flow naturally from there.`,
                ].join('\n');
            }

            await commands.openclawSendMessage(effectiveSessionKey, contextMsg, true);
        } catch (e) {
            console.error(e);
            setIsSending(false);
            setActiveRun(null);
        }
    }, [effectiveSessionKey, gatewayRunning]);

    useEffect(() => {
        isUserScrolling.current = false;
        fetchHistory();

        // The backend boot inject fires ~1.5s after engine start.
        // If the user navigates to LiveChat while the boot response is
        // still processing (LLM call in flight), the initial fetchHistory()
        // won't have it yet and the streaming events were missed because
        // no listener was mounted. Re-fetch after a delay to catch it.
        if (gatewayRunning && isCoreView) {
            const timer = setTimeout(() => fetchHistory(), 4000);
            return () => clearTimeout(timer);
        }
    }, [fetchHistory, gatewayRunning, isCoreView]);

    // NOTE: Bootstrap auto-trigger was removed.
    // The backend boot inject (ironclaw_bridge.rs::start()) now handles both
    // BOOTSTRAP and SESSION_START injection automatically when the engine starts.
    // This eliminates the double-firing race between frontend auto-trigger and
    // backend boot inject. The manual button in Console tab remains for manual
    // re-trigger if needed.

    // NOTE: Auto-inject date context was removed.
    // It raced with user messages causing "Turn in progress" errors.
    // The user can manually refresh context via the Wake Up button.

    useEffect(() => {
        if (!effectiveSessionKey) return;
        // Listen for ALL openclaw events — don't gate on gatewayRunning
        // so we never miss events during the polling interval gap.
        const unlistenPromise = listen<any>('openclaw-event', (event) => {
            const uiEvent = event.payload;

            // ── Skip events owned by other panels ────────────────────────
            // LogEntry events are consumed by the Logs tab; RoutineLifecycle
            // by the Automations panel. Don't process them here.
            if (uiEvent.kind === 'LogEntry') return;

            // ── Handle global events (no session_key) ────────────────────
            if (uiEvent.kind === 'Error') {
                const msg = uiEvent.message || 'Unknown engine error';
                toast.error(`🔴 Engine Error: ${msg}`, { duration: 8000 });
                setMessages(prev => [...prev, {
                    id: `error-${Date.now()}`,
                    role: 'system',
                    ts_ms: Date.now(),
                    text: `⚠️ Engine Error: ${msg} (code: ${uiEvent.code || 'unknown'})`,
                    source: 'openclaw',
                    metadata: { type: 'error' }
                }]);
                return;
            }
            if (uiEvent.kind === 'Disconnected') {
                toast.error(`Gateway disconnected: ${uiEvent.reason || 'unknown'}`, { duration: 5000 });
                setIsSending(false);
                setActiveRun(null);
                return;
            }
            if (uiEvent.kind === 'BootstrapCompleted') {
                // Agent deleted BOOTSTRAP.md — mark done in identity.json and refresh parent.
                commands.openclawSetBootstrapCompleted(true).catch(() => { });
                onBootstrapComplete?.();
                toast.success('Identity ritual complete — agent is fully initialized! 🎉', { duration: 6000 });
                return;
            }
            if (uiEvent.kind === 'FileCreated') {
                const { path, relative_path, bytes } = uiEvent;
                const displayName = relative_path || path.split('/').pop() || path;
                const kb = bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`;
                // Show persistent toast with Finder link
                toast.success(`📄 File created: ${displayName} (${kb})`, {
                    duration: 8000,
                    action: {
                        label: 'Reveal',
                        onClick: () => {
                            (window as any).__tauri__?.invoke('openclaw_reveal_file', { path }).catch(() => { });
                        },
                    },
                });
                // Also inject a system message card into the chat so it's permanent
                setMessages(prev => [...prev, {
                    id: `file-created-${Date.now()}`,
                    role: 'system' as const,
                    ts_ms: Date.now(),
                    text: `📄 **File created:** \`${displayName}\` (${kb})`,
                    source: 'openclaw',
                    metadata: {
                        type: 'file_created',
                        absolute_path: path,
                        relative_path,
                        bytes,
                    },
                }]);
                return;
            }
            if (uiEvent.kind === 'RoutineLifecycle') {
                const { routine_name, event: evType, result_summary } = uiEvent as any;
                const msgId = `routine-${evType}-${routine_name}-${Date.now()}`;
                const isHeartbeat = routine_name === '__heartbeat__';

                // ── "message" events carry live output from emit_user_message ──
                if (evType === 'message' && result_summary) {
                    const content = String(result_summary).replace(/^\[(progress|interim_result|warning|question)\]\s*/i, '');

                    // Skip placeholder/noise messages from the planner pre-fill
                    if (content.includes('[Bullet') || content.includes('[placeholder') || content.length < 20) {
                        return;
                    }

                    // For heartbeat, replace existing heartbeat messages to avoid chat clutter
                    if (isHeartbeat) {
                        setMessages(prev => {
                            const filtered = prev.filter(m =>
                                !(m.metadata?.type === 'routine_message' && m.metadata?.routine_name === '__heartbeat__')
                            );
                            return [...filtered, {
                                id: msgId,
                                role: 'assistant' as const,
                                ts_ms: Date.now(),
                                text: content,
                                source: 'openclaw',
                                metadata: {
                                    type: 'automation_card',
                                    routine_name,
                                    variant: 'heartbeat',
                                    status: 'running',
                                },
                            }];
                        });
                    } else {
                        setMessages(prev => [...prev, {
                            id: msgId,
                            role: 'assistant' as const,
                            ts_ms: Date.now(),
                            text: `🤖 *Automation "${routine_name}":*\n\n${content}`,
                            source: 'openclaw',
                            metadata: { type: 'routine_message', routine_name },
                        }]);
                    }
                    if (!isUserScrolling.current) scrollToBottom();
                    return;
                }

                // ── "attention" — heartbeat found items needing attention ──────
                if (evType === 'attention') {
                    setMessages(prev => {
                        // Remove any interim heartbeat messages
                        const filtered = prev.filter(m =>
                            !(m.metadata?.type === 'automation_card' && m.metadata?.routine_name === '__heartbeat__' && m.metadata?.status === 'running')
                        );
                        return [...filtered, {
                            id: msgId,
                            role: 'assistant' as const,
                            ts_ms: Date.now(),
                            text: result_summary || '',
                            source: 'openclaw',
                            metadata: {
                                type: 'automation_card',
                                routine_name,
                                variant: isHeartbeat ? 'heartbeat' : 'automation',
                                status: 'attention',
                            },
                        }];
                    });
                    toast('🔔 Heartbeat: items need attention', { duration: 6000, icon: '💓' });
                    if (!isUserScrolling.current) scrollToBottom();
                    return;
                }

                // ── "dispatched" — non-heartbeat automations open SubAgentPanel ──
                if (evType === 'dispatched' && !isHeartbeat) {
                    setSubAgentPanelOpen(true);
                    setSubAgentPanelDismissed(false);
                    toast.info(`🤖 Automation "${routine_name}" running as sub-agent`, { duration: 5000 });
                    return;
                }

                // ── "completed" — show AutomationCard with results ────────────
                if (evType === 'completed' && result_summary && result_summary !== 'Job completed successfully') {
                    setMessages(prev => {
                        // Remove interim heartbeat messages for this routine
                        const filtered = prev.filter(m =>
                            !(m.metadata?.routine_name === routine_name && (m.metadata?.status === 'running' || m.metadata?.type === 'routine_message'))
                        );
                        return [...filtered, {
                            id: msgId,
                            role: 'assistant' as const,
                            ts_ms: Date.now(),
                            text: result_summary,
                            source: 'openclaw',
                            metadata: {
                                type: 'automation_card',
                                routine_name,
                                variant: isHeartbeat ? 'heartbeat' : 'automation',
                                status: 'ok',
                            },
                        }];
                    });
                    toast.success(`✅ Automation "${routine_name}" completed`, { duration: 6000 });
                    if (!isUserScrolling.current) scrollToBottom();
                    return;
                }

                // ── "failed" — show AutomationCard with error ─────────────────
                if (evType === 'failed') {
                    setMessages(prev => {
                        const filtered = prev.filter(m =>
                            !(m.metadata?.routine_name === routine_name && (m.metadata?.status === 'running' || m.metadata?.type === 'routine_message'))
                        );
                        return [...filtered, {
                            id: msgId,
                            role: 'assistant' as const,
                            ts_ms: Date.now(),
                            text: result_summary || 'Automation failed',
                            source: 'openclaw',
                            metadata: {
                                type: 'automation_card',
                                routine_name,
                                variant: isHeartbeat ? 'heartbeat' : 'automation',
                                status: 'failed',
                            },
                        }];
                    });
                    const summarySnippet = result_summary ? ` — ${String(result_summary).slice(0, 120)}` : '';
                    toast.error(`❌ Automation "${routine_name}" failed${summarySnippet}`, { duration: 8000 });
                    if (!isUserScrolling.current) scrollToBottom();
                    return;
                }

                // ── Fallback for other events (started, dispatched for heartbeat) ──
                const summarySnippet = result_summary ? ` — ${String(result_summary).slice(0, 120)}` : '';
                const textMap: Record<string, string> = {
                    started: `⏱ Automation **${routine_name}** started`,
                    dispatched: `🔄 Automation **${routine_name}** dispatched`,
                    completed: `✅ Automation **${routine_name}** completed${summarySnippet}`,
                    failed: `❌ Automation **${routine_name}** failed${summarySnippet}`,
                };
                const text = textMap[evType] ?? `🔄 Automation **${routine_name}**: ${evType}`;
                setMessages(prev => [...prev, {
                    id: msgId,
                    role: 'system' as const,
                    ts_ms: Date.now(),
                    text,
                    source: 'openclaw',
                    metadata: { type: 'routine_lifecycle', routine_name, event: evType, summary: result_summary },
                }]);
                if (evType === 'started') toast.info(`⏱ Automation "${routine_name}" started`, { duration: 4000 });
                return;
            }
            if (uiEvent.kind === 'FactoryReset') {

                // Clear all cached frontend state — backend DB has been wiped
                setMessages([]);
                setIsSending(false);
                setActiveRun(null);
                setCurrentRunId(null);
                ocStreamStartRef.current = null;
                ocCharsReceivedRef.current = 0;
                ocActiveMessageIdRef.current = null;
                // Notify parent to re-check bootstrap state from identity.json
                // (backend has now set bootstrap_completed=false)
                onFactoryReset?.();
                return;
            }

            // ── Session-scoped events ────────────────────────────────────
            if (uiEvent.session_key !== effectiveSessionKey) return;

            // Handle message events
            if (['AssistantInternal', 'AssistantSnapshot', 'AssistantDelta', 'AssistantFinal', 'ToolUpdate', 'RunStatus'].includes(uiEvent.kind)) {
                updateMessagesFromEvent(uiEvent);
                if (!isUserScrolling.current) {
                    scrollToBottom();
                }
            }

            // Track active run for LiveAgentStatus
            if (uiEvent.kind === 'RunStatus') {
                const lowerStatus = uiEvent.status?.toLowerCase?.() ?? '';
                const TERMINAL_STATUSES = ['ok', 'error', 'aborted', 'done', 'interrupted', 'rejected'];

                if (TERMINAL_STATUSES.includes(lowerStatus)) {
                    // ── Run finished ──
                    setIsSending(false);
                    setCurrentRunId(null);

                    const errorMsg = uiEvent.error || null;

                    setActiveRun(prev => prev ? {
                        ...prev,
                        status: (lowerStatus === 'ok' || lowerStatus === 'done') ? 'completed' : 'failed',
                        error: errorMsg || prev.error,
                        completedAt: Date.now()
                    } : null);

                    // Surface RunStatus errors via toast AND inject into chat
                    if (lowerStatus === 'error' && errorMsg) {
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

                    // Clear after delay so LiveAgentStatus can show completion
                    setTimeout(() => setActiveRun(null), errorMsg ? 8000 : 3000);
                } else {
                    // ── Run is active (started, in_flight, compacting, awaiting approval, etc.) ──
                    setIsSending(true);
                    const rid = uiEvent.run_id || `run-${Date.now()}`;
                    setCurrentRunId(rid);
                    setActiveRun(prev => {
                        if (prev && prev.id === rid) return { ...prev, status: 'running' };
                        return { id: rid, text: '', tools: [], approvals: [], status: 'running', startedAt: Date.now() };
                    });

                    // Reset speed tracking for new run
                    if (!ocStreamStartRef.current) {
                        ocStreamStartRef.current = null;
                        ocCharsReceivedRef.current = 0;
                        ocActiveMessageIdRef.current = null;
                    }
                }
            }

            // Auto-activate processing indicator from Thinking events
            if (uiEvent.kind === 'AssistantInternal') {
                setActiveRun(prev => {
                    if (prev) return prev;
                    const rid = uiEvent.run_id || `run-${Date.now()}`;
                    setIsSending(true);
                    setCurrentRunId(rid);
                    return { id: rid, text: '', tools: [], approvals: [], status: 'running', startedAt: Date.now() };
                });
            }

            // Accumulate tool data into activeRun (auto-create if needed)
            if (uiEvent.kind === 'ToolUpdate') {
                setActiveRun(prev => {
                    const rid = uiEvent.run_id || currentRunId || `run-${Date.now()}`;
                    // Auto-create activeRun if it doesn't exist yet
                    if (!prev) {
                        setIsSending(true);
                        setCurrentRunId(rid);
                        prev = { id: rid, text: '', tools: [], approvals: [], status: 'running', startedAt: Date.now() };
                    }
                    // Find the last tool with the same name that isn't already completed/failed
                    // (allows stream/started → ok/error transitions)
                    let existingIdx = -1;
                    for (let i = prev.tools.length - 1; i >= 0; i--) {
                        if (prev.tools[i].tool === uiEvent.tool_name && prev.tools[i].status !== 'completed' && prev.tools[i].status !== 'failed') {
                            existingIdx = i;
                            break;
                        }
                    }
                    const newStatus = uiEvent.status === 'ok' ? 'completed' as const :
                        uiEvent.status === 'error' ? 'failed' as const :
                            uiEvent.status === 'started' ? 'started' as const : 'running' as const;

                    // Helper to filter out null/undefined/"null" for display
                    const cleanValue = (v: any) => (v === null || v === undefined || v === 'null' || v === 'Null') ? undefined : v;

                    if (existingIdx >= 0) {
                        const updatedTools = [...prev.tools];
                        updatedTools[existingIdx] = {
                            ...updatedTools[existingIdx],
                            status: newStatus,
                            input: cleanValue(uiEvent.input) ?? updatedTools[existingIdx].input,
                            output: cleanValue(uiEvent.output) ?? updatedTools[existingIdx].output,
                        };
                        return { ...prev, tools: updatedTools };
                    }
                    return { ...prev, tools: [...prev.tools, { tool: uiEvent.tool_name, input: cleanValue(uiEvent.input), output: cleanValue(uiEvent.output), status: newStatus, timestamp: Date.now() }] };
                });
            }

            // Accumulate text into activeRun + track speed
            if (uiEvent.kind === 'AssistantDelta') {
                const delta = uiEvent.delta || '';
                setActiveRun(prev => prev ? { ...prev, text: prev.text + delta } : prev);

                // Speed tracking
                if (delta.length > 0) {
                    if (!ocStreamStartRef.current) {
                        ocStreamStartRef.current = Date.now();
                    }
                    ocCharsReceivedRef.current += delta.length;
                    ocActiveMessageIdRef.current = uiEvent.message_id;

                    const elapsed = (Date.now() - ocStreamStartRef.current) / 1000;
                    if (elapsed > 0.3) {
                        const tokPerSec = Math.round((ocCharsReceivedRef.current / 4 / elapsed) * 10) / 10;
                        setMessages(prev => prev.map(m =>
                            m.id === uiEvent.message_id ? { ...m, tokensPerSec: tokPerSec } : m
                        ));
                    }
                }
            } else if (uiEvent.kind === 'AssistantSnapshot' || uiEvent.kind === 'AssistantFinal') {
                setActiveRun(prev => prev ? { ...prev, text: uiEvent.text || '' } : prev);

                // Stamp final speed on completion
                if (uiEvent.kind === 'AssistantFinal' && ocStreamStartRef.current && ocCharsReceivedRef.current > 0) {
                    const elapsed = (Date.now() - ocStreamStartRef.current) / 1000;
                    if (elapsed > 0.1) {
                        const tokPerSec = Math.round((ocCharsReceivedRef.current / 4 / elapsed) * 10) / 10;
                        const msgId = ocActiveMessageIdRef.current || uiEvent.message_id;
                        setMessages(prev => prev.map(m =>
                            m.id === msgId ? { ...m, tokensPerSec: tokPerSec } : m
                        ));
                    }
                }
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
    }, [effectiveSessionKey, scrollToBottom]);

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
                const toolMsgId = `tool-${uiEvent.tool_name}-${uiEvent.run_id}`;
                const existing = prev.find(m => m.id === toolMsgId);
                let content = `[Tool Call: ${uiEvent.tool_name}]`;
                const metadata = { type: 'tool', name: uiEvent.tool_name, status: uiEvent.status, input: uiEvent.input, output: uiEvent.output, run_id: uiEvent.run_id };

                // Detect bootstrap ritual completion: agent deleted BOOTSTRAP.md
                if (
                    uiEvent.tool_name === 'memory_delete' &&
                    uiEvent.status === 'completed' &&
                    typeof uiEvent.input === 'object' &&
                    (uiEvent.input?.path ?? '').includes('BOOTSTRAP')
                ) {
                    // Fire slightly delayed so the final agent message renders first
                    setTimeout(() => onBootstrapComplete(), 1500);
                }

                if (existing) return prev.map(m => m.id === toolMsgId ? { ...m, text: content, metadata } : m);
                return [...prev, { id: toolMsgId, role: 'system', ts_ms: Date.now(), text: content, source: 'openclaw', metadata }];
            }
            return prev;
        });
    }

    const handleToggleThinking = async () => {
        const next = !thinkingEnabled;
        try {
            await openclaw.setThinking(next, next ? thinkingBudget : undefined);
            setThinkingEnabled(next);
            toast.success(next ? '🧠 Thinking mode enabled (native)' : 'Thinking mode disabled');
        } catch (e) {
            console.error('Failed to set thinking mode:', e);
            toast.error('Failed to set thinking mode');
        }
    };

    const [exportFormat, setExportFormat] = useState<'md' | 'json' | 'txt' | 'csv' | 'html'>('md');
    const [showExportMenu, setShowExportMenu] = useState(false);

    const EXPORT_FORMATS = [
        { id: 'md' as const, label: 'Markdown', ext: '.md' },
        { id: 'json' as const, label: 'JSON', ext: '.json' },
        { id: 'txt' as const, label: 'Plain Text', ext: '.txt' },
        { id: 'csv' as const, label: 'CSV', ext: '.csv' },
        { id: 'html' as const, label: 'HTML', ext: '.html' },
    ];

    const handleExportSession = async (format?: 'md' | 'json' | 'txt' | 'csv' | 'html') => {
        if (!effectiveSessionKey) return;
        const fmt = format || exportFormat;
        try {
            const result = await openclaw.exportSession(effectiveSessionKey, fmt);
            if (fmt === 'md' || fmt === 'txt') {
                await navigator.clipboard.writeText(result.transcript);
                toast.success(`Exported ${result.message_count} messages (${fmt.toUpperCase()}) to clipboard`);
            } else {
                const blob = new Blob([result.transcript], { type: 'text/plain' });
                const url = URL.createObjectURL(blob);
                const a = document.createElement('a');
                a.href = url;
                a.download = `session-export${EXPORT_FORMATS.find(f => f.id === fmt)?.ext || '.txt'}`;
                a.click();
                URL.revokeObjectURL(url);
                toast.success(`Downloaded ${result.message_count} messages as ${fmt.toUpperCase()}`);
            }
        } catch (e) {
            toast.error('Failed to export session');
        }
        setShowExportMenu(false);
    };

    const handleSend = async () => {
        if (!input.trim() || !effectiveSessionKey) return;
        const msg = input.trim();
        setInput('');
        // Don't block on isSending — the engine queues messages via idempotency keys.
        // The user should be able to send follow-up messages while the agent processes.
        setIsSending(true);
        // Optimistic update
        const optimisticMsg: OpenClawMessage = { id: `temp-${Date.now()}`, role: 'user', ts_ms: Date.now(), text: msg, source: 'openclaw' };
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

        try {
            // Backend handles the full lifecycle: abort → wait → delete → reset → retry
            await openclaw.deleteOpenClawSession(sessionKey);
            toast.success("Session deleted", { id: tId });
            if (onNavigateToSettings) onNavigateToSettings('openclaw-gateway');
        } catch (e: any) {
            toast.error("Failed to delete session: " + (e?.message || String(e)), { id: tId });
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
        <div className="flex-1 flex relative h-full overflow-hidden bg-background">
            {/* Main chat column */}
            <div className={cn(
                "flex-1 flex flex-col relative h-full overflow-hidden transition-all duration-300",
                subAgentPanelOpen ? 'mr-0' : ''
            )}>
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
                            <div className="relative">
                                <div className="flex items-center">
                                    <button
                                        onClick={() => handleExportSession()}
                                        className="p-2 rounded-l-lg hover:bg-muted text-muted-foreground hover:text-foreground transition-colors"
                                        title={`Export as ${exportFormat.toUpperCase()}`}
                                    >
                                        <Download className="w-4 h-4" />
                                    </button>
                                    <button
                                        onClick={() => setShowExportMenu(!showExportMenu)}
                                        className="p-2 rounded-r-lg hover:bg-muted text-muted-foreground hover:text-foreground transition-colors border-l border-white/5"
                                        title="Choose format"
                                    >
                                        <ChevronDown className="w-3 h-3" />
                                    </button>
                                </div>
                                {showExportMenu && (
                                    <div className="absolute top-full right-0 mt-1 p-1 bg-zinc-900 border border-border rounded-xl shadow-2xl z-50 min-w-[140px] animate-in fade-in zoom-in-95 duration-150">
                                        {EXPORT_FORMATS.map(f => (
                                            <button
                                                key={f.id}
                                                onClick={() => { setExportFormat(f.id); handleExportSession(f.id); }}
                                                className={cn(
                                                    "w-full text-left px-3 py-1.5 rounded-lg text-xs font-medium transition-colors flex items-center gap-2",
                                                    exportFormat === f.id ? "bg-primary/15 text-primary" : "text-muted-foreground hover:text-foreground hover:bg-white/5"
                                                )}
                                            >
                                                <FileDown className="w-3 h-3" />
                                                {f.label}
                                            </button>
                                        ))}
                                    </div>
                                )}
                            </div>
                        )}
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
                            <div className="flex p-1 bg-background/60 backdrop-blur-xl rounded-2xl pointer-events-auto border border-border/40 shadow-2xl">
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
                                                    if (msg.text.includes('SYSTEM_CONTEXT_REFRESH')) return false;
                                                    if (msg.text.includes('[SYSTEM_CONTEXT_UPDATE]')) return false;
                                                    if (msg.text.trim().startsWith('[Tool Call:')) return false;
                                                    if (msg.text.includes('Pre-compaction memory flush')) return false;
                                                    if (msg.text.includes('Store durable memories now')) return false;
                                                    if (msg.text.includes('NO_REPL')) return false;

                                                    // Hide assistant messages that are purely tool calls
                                                    if (msg.role === 'assistant' && msg.text.includes('[TOOL_CALLS]')) {
                                                        // Check if there's any real content besides tool calls
                                                        const withoutToolCalls = msg.text.replace(/\[TOOL_CALLS\]\w+\[ARGS\]\{.*?\}[\s]*/gm, '').trim();
                                                        if (!withoutToolCalls) return false;
                                                    }


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
                                                        {/* AutomationCard gets its own layout — no bot avatar or bubble */}
                                                        {msg.metadata?.type === 'automation_card' ? (
                                                            <div className="w-full max-w-[85%]">
                                                                <AutomationCard
                                                                    routineName={msg.metadata.routine_name || ''}
                                                                    variant={msg.metadata.variant || 'automation'}
                                                                    status={msg.metadata.status || 'ok'}
                                                                    content={msg.text}
                                                                    timestamp={msg.ts_ms}
                                                                />
                                                            </div>
                                                        ) : (
                                                            <>
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
                                                                    {msg.role === 'system'
                                                                        ? <SystemMessageContent text={msg.text} metadata={msg.metadata} onViewSession={onViewSession} />
                                                                        : <AssistantMessageContent text={msg.text} />
                                                                    }
                                                                    <div className={cn("flex items-center gap-3 mt-2 text-[10px] opacity-0 group-hover:opacity-100 uppercase transition-opacity duration-200", msg.role === 'user' ? "text-primary-foreground/60" : "text-muted-foreground/60")}>
                                                                        <span><Clock className="w-3 h-3 inline mr-1" /> {formatTime(msg.ts_ms)}</span>
                                                                        {msg.role === 'assistant' && msg.tokensPerSec != null && msg.tokensPerSec > 0 && (
                                                                            <span className="flex items-center gap-1 text-emerald-400/70">
                                                                                <Zap className="w-2.5 h-2.5" />
                                                                                {msg.tokensPerSec} tok/s
                                                                            </span>
                                                                        )}
                                                                        {msg.role !== 'user' && (
                                                                            <CopyMessageButton text={msg.text} />
                                                                        )}
                                                                    </div>
                                                                </div>
                                                                {msg.role === 'user' && <div className="w-8 h-8 rounded-xl bg-muted flex items-center justify-center shrink-0 mt-1"><User className="w-4 h-4 text-muted-foreground" /></div>}
                                                            </>
                                                        )}
                                                    </motion.div>
                                                );
                                            });
                                        })()}
                                    </AnimatePresence>

                                    {/* CHAT TAB: Empty state — agent is booting */}
                                    {isCoreView && coreTab === 'chat' && messages.length === 0 && !isLoading && gatewayRunning && (
                                        <div className="flex flex-col items-center justify-center py-20 gap-5">
                                            <div className="w-16 h-16 rounded-full bg-white/5 flex items-center justify-center relative">
                                                <div className="absolute inset-0 rounded-full border border-emerald-500/20 animate-ping" />
                                                <Bot className="w-8 h-8 text-emerald-500" />
                                            </div>
                                            <div className="text-center">
                                                <h3 className="text-lg font-medium text-white">
                                                    {bootstrapNeeded ? 'Awakening…' : 'Coming online…'}
                                                </h3>
                                                <p className="text-sm text-gray-500 mt-1">
                                                    {bootstrapNeeded
                                                        ? 'Your agent is waking up for the first time.'
                                                        : 'Your agent is preparing to greet you.'}
                                                </p>
                                            </div>
                                        </div>
                                    )}

                                    {/* CORE VIEW: Console tab controls */}
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
                                                    onClick={() => handleWakeUp(bootstrapNeeded)}
                                                    className="px-4 py-2 bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-400 text-xs font-mono uppercase tracking-wider rounded border border-emerald-500/20 transition-all flex items-center justify-center gap-2 mx-auto"
                                                >
                                                    <Zap className="w-3.5 h-3.5" />
                                                    {bootstrapNeeded ? 'Trigger Boot Sequence' : 'Refresh Context'}
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
                                        <div className="relative">
                                            <button
                                                onClick={handleToggleThinking}
                                                onContextMenu={(e) => { e.preventDefault(); setShowThinkingSlider(!showThinkingSlider); }}
                                                title={thinkingEnabled ? 'Thinking mode ON (native) — click to disable, right-click for budget' : 'Enable thinking mode (right-click for budget)'}
                                                className={cn(
                                                    "p-2 rounded-xl transition-all border",
                                                    thinkingEnabled
                                                        ? "bg-violet-500/15 text-violet-500 border-violet-500/30 shadow-sm"
                                                        : "bg-transparent text-muted-foreground border-transparent hover:bg-muted/50 hover:text-foreground"
                                                )}
                                            >
                                                <Brain className="w-4 h-4" />
                                            </button>
                                            <AnimatePresence>
                                                {showThinkingSlider && (
                                                    <motion.div
                                                        initial={{ opacity: 0, y: 8 }}
                                                        animate={{ opacity: 1, y: 0 }}
                                                        exit={{ opacity: 0, y: 8 }}
                                                        className="absolute bottom-12 right-0 bg-background/95 backdrop-blur-xl border border-border/40 rounded-xl p-3 shadow-2xl z-50 w-52"
                                                    >
                                                        <div className="flex items-center gap-2 mb-2">
                                                            <Sliders className="w-3 h-3 text-violet-400" />
                                                            <span className="text-[10px] font-bold text-violet-400 uppercase tracking-wider">Thinking Budget</span>
                                                        </div>
                                                        <input
                                                            type="range"
                                                            min={1024}
                                                            max={32768}
                                                            step={1024}
                                                            value={thinkingBudget}
                                                            onChange={(e) => setThinkingBudget(Number(e.target.value))}
                                                            onMouseUp={async () => {
                                                                if (thinkingEnabled) {
                                                                    try {
                                                                        await openclaw.setThinking(true, thinkingBudget);
                                                                        toast.success(`Budget: ${thinkingBudget.toLocaleString()} tokens`);
                                                                    } catch { }
                                                                }
                                                            }}
                                                            className="w-full accent-violet-500 h-1"
                                                        />
                                                        <div className="flex justify-between mt-1">
                                                            <span className="text-[9px] text-muted-foreground">1K</span>
                                                            <span className="text-[10px] font-mono text-violet-400">{(thinkingBudget / 1024).toFixed(0)}K tokens</span>
                                                            <span className="text-[9px] text-muted-foreground">32K</span>
                                                        </div>
                                                    </motion.div>
                                                )}
                                            </AnimatePresence>
                                        </div>
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
            </div>

            {/* Right-side Sub-Agent Panel */}
            <AnimatePresence>
                {subAgentPanelOpen && sessionKey && (
                    <motion.div
                        initial={{ width: 0, opacity: 0 }}
                        animate={{ width: 340, opacity: 1 }}
                        exit={{ width: 0, opacity: 0 }}
                        transition={{ duration: 0.3, ease: [0.25, 0.1, 0.25, 1] }}
                        className="h-full border-l border-zinc-700/40 overflow-hidden shrink-0"
                    >
                        <SubAgentPanel
                            sessionKey={sessionKey}
                            onViewSession={onViewSession}
                            onClose={() => {
                                setSubAgentPanelOpen(false);
                                setSubAgentPanelDismissed(true);
                            }}
                        />
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Floating button to re-open panel when dismissed but sub-agents exist */}
            {!subAgentPanelOpen && subAgentCount > 0 && (
                <button
                    onClick={() => {
                        setSubAgentPanelOpen(true);
                        setSubAgentPanelDismissed(false);
                    }}
                    className="absolute right-4 top-16 z-30 flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg
                           bg-blue-500/10 border border-blue-500/30 text-blue-400 text-[11px] font-medium
                           hover:bg-blue-500/20 transition-all backdrop-blur-sm shadow-lg"
                    title="Show sub-agent panel"
                >
                    <PanelRight className="w-3.5 h-3.5" />
                    {subAgentCount} sub-agent{subAgentCount !== 1 ? 's' : ''}
                </button>
            )}
        </div>
    );
}
