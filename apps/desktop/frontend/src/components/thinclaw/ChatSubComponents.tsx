import { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { RefreshCw, AlertTriangle, Brain, Terminal, Loader2, CheckCircle2, XCircle, Layers, ChevronRight, ChevronDown, ExternalLink, Copy, Check } from 'lucide-react';
import { cn } from '../../lib/utils';
import { ThinClawMessage } from '../../lib/thinclaw';
import { thinclawCommands } from '../../lib/generated/thinclaw-commands';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

export interface RichToolCardProps {
    name: string;
    status: 'started' | 'completed' | 'failed' | 'in_flight';
    input?: any;
    output?: any;
    isSubAgent?: boolean;
    variant?: 'live' | 'history';
    onViewSession?: (sessionKey: string) => void;
}

export function RichToolCard({ name, status, input: rawInput, output: rawOutput, isSubAgent, variant = 'live', onViewSession }: RichToolCardProps) {
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
                    className="flex items-center gap-2 w-full text-left py-1 hover:bg-muted/50 transition-colors rounded group pl-2 border-l border-border/30"
                >
                    <div className="w-4 h-4 rounded flex items-center justify-center bg-muted/70 group-hover:bg-muted transition-colors">
                        {isSubAgent ? <RefreshCw className="w-2.5 h-2.5 text-purple-400" /> : <Terminal className="w-2.5 h-2.5 text-gray-500" />}
                    </div>
                    <div className="flex-1 flex items-center gap-2">
                        <span className="text-[10px] font-medium text-muted-foreground group-hover:text-foreground transition-colors font-mono">
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
                            className="overflow-hidden ml-7 space-y-2 mt-1 mb-2 border-l-2 border-border/30 pl-3"
                        >
                            {input && (
                                <div>
                                    <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-0.5">Input</div>
                                    <pre className="text-[10px] font-mono text-muted-foreground overflow-x-auto whitespace-pre-wrap bg-muted/30 p-2 rounded">
                                        {typeof input === 'string' ? input : JSON.stringify(input, null, 2)}
                                    </pre>
                                </div>
                            )}
                            {output && (
                                <div>
                                    <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-0.5">Output</div>
                                    <pre className="text-[10px] font-mono text-muted-foreground/70 overflow-x-auto whitespace-pre-wrap bg-muted/30 p-2 rounded">
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
                    "flex items-center gap-2 w-full text-left py-1 mb-1 transition-colors rounded hover:bg-muted/50",
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
                            <div className="bg-muted/30 rounded p-2 border border-border/40">
                                <div className="text-[9px] uppercase text-muted-foreground font-semibold mb-1">Input</div>
                                <pre className="text-[10px] font-mono text-foreground/80 overflow-x-auto whitespace-pre-wrap">
                                    {typeof input === 'string' ? input : JSON.stringify(input, null, 2)}
                                </pre>
                            </div>
                        )}
                        {output && (
                            <div className="bg-muted/30 rounded p-2 border border-border/40">
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
                                        <pre className="text-[10px] font-mono text-emerald-600 dark:text-green-300/80 overflow-x-auto whitespace-pre-wrap">
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
export function extractSessionKey(output: any): string | null {
    if (!output) return null;
    let data = output;
    if (typeof output === 'string') {
        try {
            data = JSON.parse(output);
        } catch {
            // IC-005: Regex fallback for non-JSON output containing a session key
            const match = output.match(/(?:session[_-]?(?:key|id))\s*[:=]\s*["']?([a-f0-9-]{8,}|agent:\S+)["']?/i);
            if (match) return match[1];
            console.warn('[extractSessionKey] Could not parse output:', output.slice(0, 200));
            return null;
        }
    }
    // Common patterns from sessions_spawn output
    return data?.sessionKey || data?.session_key || data?.sessionId || data?.session_id || null;
}

// Shared markdown components for ThinClaw chat — handles link clicks
// by opening URLs in the system browser (Safari/Chrome) instead of
// navigating the Tauri webview.
export const markdownComponents = {
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
export function AssistantMessageContent({ text }: { text: string }) {
    // IC-010: Use non-global regex for test (no lastIndex state)
    const hasToolCalls = /\[TOOL_CALLS\]\w+\[ARGS\]\{/.test(text);

    if (!hasToolCalls) {
        return <div className="prose prose-sm dark:prose-invert text-foreground prose-headings:text-foreground prose-p:text-foreground prose-strong:text-foreground prose-li:text-foreground"><ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{text}</ReactMarkdown></div>;
    }

    // IC-010 + IC-029: Fresh global regex per render; greedy match to EOL for nested JSON
    const toolCallRegex = /\[TOOL_CALLS\](\w+)\[ARGS\](\{.*\})$/gm;

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
                    return <div key={i} className="prose prose-sm dark:prose-invert text-foreground prose-headings:text-foreground prose-p:text-foreground prose-strong:text-foreground prose-li:text-foreground"><ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{part.content}</ReactMarkdown></div>;
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
export function ToolHistoryGroup({ messages, onViewSession }: { messages: ThinClawMessage[]; onViewSession?: (sessionKey: string) => void }) {
    const [expanded, setExpanded] = useState(false);
    const count = messages.length;
    // IC-004: Use metadata.status only — string heuristics cause false positives
    const hasFailures = messages.some(m => m.metadata?.status === 'failed');

    return (
        <div className="w-full my-2">
            <button
                onClick={() => setExpanded(!expanded)}
                className={cn(
                    "flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg transition-all border",
                    expanded ? "bg-muted/30 border-border/40" : "bg-transparent border-transparent hover:bg-muted/30",
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
                        <span className="text-xs font-medium text-foreground/80">
                            Executed {count} tool{count > 1 ? 's' : ''}
                        </span>
                        {hasFailures && <span className="text-[9px] uppercase font-bold text-red-500 bg-red-500/10 px-1.5 rounded">Issues Found</span>}
                    </div>
                </div>
                {expanded ? <ChevronDown className="w-4 h-4 text-muted-foreground" /> : <ChevronRight className="w-4 h-4 text-muted-foreground" />}
            </button>
            <AnimatePresence>
                {expanded && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: "auto", opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden pl-4 pr-1 py-2 space-y-1"
                        style={{ maxHeight: 600 }}  // IC-030: prevent Safari height:auto flicker
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

export function CopyMessageButton({ text }: { text: string }) {
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

export function SystemMessageContent({ text, metadata, onViewSession }: { text: string; metadata?: any; onViewSession?: (sessionKey: string) => void }) {
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
                        // IC-009: Use typed Specta binding instead of raw __tauri__ invoke
                        thinclawCommands.thinclawRevealFile(metadata.absolute_path).catch(() => { });
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
                <p className="whitespace-pre-wrap leading-relaxed text-[11px] font-mono text-blue-600/60 dark:text-blue-200/60 pl-4 border-l-2 border-blue-500/20 py-1 transition-colors hover:text-blue-600/80 dark:hover:text-blue-200/80">
                    {content}
                </p>
            </div>
        );
    }

    // 5. General System Message
    if (!text.includes('[Tool')) {
        const content = text.replace(/^🛠️/, '').trim();
        return (
            <p className="whitespace-pre-wrap leading-relaxed opacity-90 text-[11px] font-mono text-muted-foreground pl-6 border-l border-blue-500/20 py-1">
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
