import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    RefreshCw,
    Anchor,
    Shield,
    Clock,
    ArrowUpDown,
    ChevronDown,
    ChevronRight,
    AlertCircle,
    Zap,
    Ban,
    Plus,
    Trash2,
    Copy,
    Check,
    Eye,
    Lock,
    Globe,
    Languages,
    FileText,
    Filter,
    Sparkles,
    Code2,
    ShieldAlert,
    X,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { toast } from 'sonner';

// Uniform hook point pill style — matches the app's neutral theme
const HOOK_POINT_STYLE = 'bg-white/5 text-muted-foreground border-border/40';

const HOOK_POINT_ICONS: Record<string, React.ReactNode> = {
    beforeInbound: <Anchor className="w-3 h-3" />,
    beforeToolCall: <Zap className="w-3 h-3" />,
    beforeOutbound: <ArrowUpDown className="w-3 h-3" />,
    onSessionStart: <ChevronRight className="w-3 h-3" />,
    onSessionEnd: <Ban className="w-3 h-3" />,
    transformResponse: <RefreshCw className="w-3 h-3" />,
    beforeAgentStart: <Shield className="w-3 h-3" />,
    beforeMessageWrite: <ArrowUpDown className="w-3 h-3" />,
    beforeLlmInput: <Zap className="w-3 h-3" />,
    afterLlmOutput: <ArrowUpDown className="w-3 h-3" />,
    beforeTranscribeAudio: <AlertCircle className="w-3 h-3" />,
};

// ============================================================================
// Hook Templates
// ============================================================================

interface HookTemplate {
    id: string;
    name: string;
    description: string;
    category: 'safety' | 'workflow' | 'security' | 'formatting';
    icon: React.ReactNode;
    color: string;
    bundle: object;
}

const HOOK_TEMPLATES: HookTemplate[] = [
    // ── Safety & Privacy ────────────────────────────────────────────────
    {
        id: 'pii-redactor',
        name: 'PII Redactor',
        description: 'Redact emails, phone numbers, SSNs, credit card numbers, and IP addresses from agent responses before they leave the system.',
        category: 'safety',
        icon: <ShieldAlert className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "pii-redactor",
                points: ["beforeOutbound", "beforeMessageWrite"],
                priority: 50,
                replacements: [
                    // Email addresses (RFC 5322 simplified)
                    { pattern: "[a-zA-Z0-9._%+\\-]+@[a-zA-Z0-9.\\-]+\\.[a-zA-Z]{2,}", replacement: "[EMAIL REDACTED]" },
                    // US/international phone numbers (with optional country code)
                    { pattern: "(?:\\+?\\d{1,3}[\\s.-]?)?\\(?\\d{3}\\)?[\\s.-]?\\d{3}[\\s.-]?\\d{4}", replacement: "[PHONE REDACTED]" },
                    // US Social Security Numbers (XXX-XX-XXXX)
                    { pattern: "\\b\\d{3}-\\d{2}-\\d{4}\\b", replacement: "[SSN REDACTED]" },
                    // Credit card numbers (13-19 digit sequences with optional spaces/dashes)
                    { pattern: "\\b(?:\\d[ -]?){13,19}\\b", replacement: "[CARD REDACTED]" },
                    // IPv4 addresses
                    { pattern: "\\b\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\b", replacement: "[IP REDACTED]" },
                    // IBAN (2 letter country + 2 check digits + up to 30 alphanumeric)
                    { pattern: "\\b[A-Z]{2}\\d{2}[A-Z0-9]{4,30}\\b", replacement: "[IBAN REDACTED]" },
                ],
            }],
        },
    },
    {
        id: 'credential-leak-blocker',
        name: 'Credential Leak Blocker',
        description: 'Detects credential patterns (API keys, tokens, passwords, connection strings) and replaces them with a redaction marker so the message still reaches the LLM with context.',
        category: 'safety',
        icon: <Lock className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "credential-redactor",
                points: ["beforeInbound"],
                priority: 30,
                failure_mode: "fail_closed",
                replacements: [
                    // OpenAI / Anthropic style keys (sk-...)
                    { pattern: "\\bsk-[a-zA-Z0-9]{20,}\\b", replacement: "[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    // AWS access keys (AKIA...)
                    { pattern: "\\bAKIA[A-Z0-9]{16}\\b", replacement: "[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    // GitHub PATs (ghp_...)
                    { pattern: "\\bghp_[a-zA-Z0-9]{36}\\b", replacement: "[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    // Slack bot tokens (xoxb-...)
                    { pattern: "\\bxoxb-[0-9\\-]{10,}\\b", replacement: "[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    // GitLab PATs (glpat-...)
                    { pattern: "\\bglpat-[a-zA-Z0-9\\-]{20,}\\b", replacement: "[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    // Plaintext password sharing ("my password is ..." or "password: ...")
                    { pattern: "(?i)(my\\s+password\\s+is\\s+)\\S+", replacement: "$1[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    { pattern: "(?i)(password:\\s*)\\S+", replacement: "$1[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    { pattern: "(?i)(secret_key\\s*[=:]\\s*)\\S+", replacement: "$1[REDACTED — CREDENTIAL LEAK BLOCKER]" },
                    // MongoDB / Postgres connection strings (contain embedded credentials)
                    { pattern: "mongodb\\+srv://[^\\s]+", replacement: "[REDACTED — CONNECTION STRING REMOVED]" },
                    { pattern: "postgres://[^\\s]+", replacement: "[REDACTED — CONNECTION STRING REMOVED]" },
                ],
            }],
        },
    },
    {
        id: 'safety-filter',
        name: 'Content Safety Filter',
        description: 'Block requests for clearly harmful activities — malware creation, exploitation, unauthorized access, or social engineering attacks.',
        category: 'safety',
        icon: <Shield className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "safety-filter",
                points: ["beforeInbound"],
                priority: 10,
                failure_mode: "fail_closed",
                when_regex: "(?i)(\\bhack\\s+into\\b|\\bexploit\\s+(a\\s+)?vulnerability\\b|\\bcreate\\s+(a\\s+)?malware\\b|\\bddos\\s+attack\\b|\\bransomware\\b|\\bphishing\\s+(email|page|site)\\b|\\bbypass\\s+(authentication|security|firewall)\\b|\\bbrute\\s*force\\s+(password|login)\\b|\\bsql\\s*injection\\s+attack\\b|\\bkeylogger\\b)",
                reject_reason: "🛑 This request has been blocked by the content safety filter. The agent cannot assist with activities that could cause harm, compromise security, or violate laws.",
            }],
        },
    },
    // ── Workflow & Productivity ──────────────────────────────────────────
    {
        id: 'language-enforcer',
        name: 'Language Enforcer',
        description: 'Force all agent responses into a specific language, regardless of the input language. Edit the language name in the config after activating.',
        category: 'workflow',
        icon: <Languages className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "language-enforcer",
                points: ["beforeLlmInput"],
                priority: 90,
                prepend: "[LANGUAGE INSTRUCTION: You MUST respond entirely in English. This applies to all parts of your response including explanations, code comments, error messages, and headers. If the user writes in another language, understand their intent but always reply in English.]\n\n",
            }],
        },
    },
    {
        id: 'concise-mode',
        name: 'Concise Mode',
        description: 'Enforce brief, focused responses — max 3 paragraphs, bullet points for lists, no unnecessary preamble.',
        category: 'workflow',
        icon: <FileText className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "concise-mode",
                points: ["beforeLlmInput"],
                priority: 95,
                append: "\n\n[RESPONSE FORMAT: Keep your response concise and scannable. Maximum 3 short paragraphs. Use bullet points for lists. Skip unnecessary introductions like 'Sure!' or 'Of course!'. Get straight to the answer. Favor code examples over lengthy explanations.]",
            }],
        },
    },
    {
        id: 'custom-signature',
        name: 'Custom Signature',
        description: 'Append a branded signature and AI disclaimer to every outbound response. Customize the text after activating.',
        category: 'workflow',
        icon: <Sparkles className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "custom-signature",
                points: ["beforeOutbound"],
                priority: 200,
                append: "\n\n---\n_Powered by ThinClaw AI · Responses are generated by AI and may contain errors · Always verify critical information_",
            }],
        },
    },
    {
        id: 'code-review-context',
        name: 'Code Quality Standards',
        description: 'When code-related tasks are detected, automatically inject comprehensive coding standards and best practices for TypeScript/React projects.',
        category: 'workflow',
        icon: <Code2 className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "code-quality-standards",
                points: ["beforeLlmInput"],
                priority: 80,
                when_regex: "(?i)(review|refactor|code|function|component|bug|fix|implement|typescript|react|javascript|python|rust|class\\b|interface\\b|module\\b|test\\b|spec\\b)",
                prepend: `[CODE QUALITY STANDARDS — Apply these when writing or reviewing code:
• TypeScript: Use strict mode, prefer \`const\` over \`let\`, avoid \`any\` — use \`unknown\` or proper generics instead
• Functions: Keep functions under 30 lines, single responsibility, descriptive names (verbs for actions, nouns for values)
• Error handling: Never swallow errors silently, use typed error handling, prefer Result/Either patterns over try/catch where possible
• React: Prefer functional components with hooks, memoize expensive computations, avoid prop drilling (use context or composition)
• Naming: camelCase for variables/functions, PascalCase for types/components, UPPER_SNAKE_CASE for constants
• Comments: Explain WHY, not WHAT — code should be self-documenting. Add JSDoc/TSDoc for public APIs
• Testing: Suggest unit tests for logic, integration tests for APIs, prefer testing behavior over implementation
• Security: Sanitize user inputs, parameterize queries, validate boundaries, never log secrets
• Performance: Avoid unnecessary re-renders, lazy load where possible, prefer \`Map\` over \`Object\` for lookups]\n\n`,
            }],
        },
    },
    // ── Security & Control ──────────────────────────────────────────────
    {
        id: 'tool-allowlist',
        name: 'Tool Restrictor',
        description: 'Block the agent from using dangerous tools — shell execution, file system mutations, code execution, and system-level commands.',
        category: 'security',
        icon: <Ban className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "tool-restrictor",
                points: ["beforeToolCall"],
                priority: 20,
                failure_mode: "fail_closed",
                when_regex: "(?i)(shell_exec|run_command|execute_command|file_delete|file_write|system_command|run_code|code_execute|process_spawn|eval\\b|exec\\b|rm\\s+-rf|sudo\\b|chmod\\b|chown\\b)",
                reject_reason: "🔒 This tool/operation has been blocked by security policy. The agent is restricted from executing shell commands, deleting files, or running arbitrary code. Contact your administrator to adjust tool permissions.",
            }],
        },
    },
    // ── Response Formatting ─────────────────────────────────────────────
    {
        id: 'url-sanitizer',
        name: 'URL Tracking Cleaner',
        description: 'Strip UTM parameters, Facebook click IDs, Google click IDs, and other tracking tokens from URLs in agent responses.',
        category: 'formatting',
        icon: <Globe className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "url-sanitizer",
                points: ["beforeOutbound"],
                priority: 150,
                replacements: [
                    { pattern: "([?&])(utm_source|utm_medium|utm_campaign|utm_term|utm_content|fbclid|gclid|mc_cid|mc_eid|ref|source)=[^&\\s]*", replacement: "" },
                    { pattern: "\\?&", replacement: "?" },
                    { pattern: "\\?$", replacement: "" },
                ],
            }],
        },
    },
    {
        id: 'markdown-stripper',
        name: 'Plain Text Mode',
        description: 'Strip markdown formatting from responses — removes code blocks, bold, italic, headers, and links. Ideal for SMS, Telegram, or simple chat integrations.',
        category: 'formatting',
        icon: <Filter className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "strip-markdown",
                points: ["beforeOutbound"],
                priority: 180,
                replacements: [
                    // Remove fenced code blocks (keep content)
                    { pattern: "```(?:\\w+)?\\n([\\s\\S]*?)```", replacement: "$1" },
                    // Remove inline code backticks
                    { pattern: "`([^`]+)`", replacement: "$1" },
                    // Bold **text** → text
                    { pattern: "\\*\\*(.+?)\\*\\*", replacement: "$1" },
                    // Italic *text* → text
                    { pattern: "\\*(.+?)\\*", replacement: "$1" },
                    // Headers: ## Heading → Heading
                    { pattern: "^#{1,6}\\s+(.+)$", replacement: "$1" },
                    // Links: [text](url) → text (url)
                    { pattern: "\\[([^\\]]+)\\]\\(([^)]+)\\)", replacement: "$1 ($2)" },
                    // Horizontal rules
                    { pattern: "^---+$", replacement: "" },
                ],
            }],
        },
    },
    {
        id: 'response-length-guard',
        name: 'Response Length Guard',
        description: 'Instruct the agent to stay within reasonable response lengths, preventing excessively verbose outputs that waste tokens and attention.',
        category: 'formatting',
        icon: <ArrowUpDown className="w-4 h-4" />,
        color: 'text-muted-foreground',
        bundle: {
            rules: [{
                name: "response-length-guard",
                points: ["beforeLlmInput"],
                priority: 92,
                append: "\n\n[IMPORTANT: Keep your total response under 800 words unless the user explicitly asks for a detailed or comprehensive answer. If the topic requires a longer response, break it into clearly labeled sections. Prefer brevity and clarity over completeness.]",
            }],
        },
    },
];

const CATEGORY_LABELS: Record<string, { label: string }> = {
    safety: { label: 'Safety & Privacy' },
    workflow: { label: 'Workflow & Productivity' },
    security: { label: 'Security & Control' },
    formatting: { label: 'Response Formatting' },
};

// ============================================================================
// Components
// ============================================================================

function HookCard({ hook, onRemove }: { hook: thinclaw.HookInfoItem; onRemove: () => void }) {
    const [expanded, setExpanded] = useState(false);
    const isFailClosed = hook.failure_mode === 'FailClosed';
    const isBuiltin = hook.name.startsWith('builtin.');

    return (
        <motion.div
            layout
            className={cn(
                "rounded-2xl border transition-all duration-300",
                "bg-white/[0.02] border-white/5 hover:border-border/40",
                "shadow-sm hover:shadow-md"
            )}
        >
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full p-5 flex items-start justify-between text-left"
            >
                <div className="flex items-center gap-3 flex-1 min-w-0">
                    <div className={cn(
                        "p-2.5 rounded-xl border transition-colors flex items-center justify-center",
                        "bg-primary/10 border-primary/20 text-primary"
                    )}>
                        <Anchor className="w-5 h-5" />
                    </div>
                    <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                            <h3 className="font-semibold text-sm truncate">{hook.name}</h3>
                            <span className="text-[10px] font-mono text-muted-foreground/60 px-1.5 py-0.5 rounded bg-white/5 border border-white/5">
                                P{hook.priority}
                            </span>
                            {isFailClosed && (
                                <span className="text-[9px] font-bold uppercase tracking-tight px-1.5 py-0.5 rounded bg-red-500/10 border border-red-500/20 text-red-400">
                                    Fail-Closed
                                </span>
                            )}
                        </div>
                        <div className="flex flex-wrap gap-1.5 mt-2">
                            {hook.hook_points.map(point => (
                                <span
                                    key={point}
                                    className={cn(
                                        "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border",
                                        HOOK_POINT_STYLE
                                    )}
                                >
                                    {HOOK_POINT_ICONS[point]}
                                    {point}
                                </span>
                            ))}
                        </div>
                    </div>
                </div>
                <div className="flex items-center gap-2 flex-none mt-1">
                    {!isBuiltin && (
                        <button
                            onClick={(e) => { e.stopPropagation(); onRemove(); }}
                            className="p-1.5 rounded-lg hover:bg-red-500/10 text-muted-foreground hover:text-red-400 transition-colors"
                            title="Remove hook"
                        >
                            <Trash2 className="w-3.5 h-3.5" />
                        </button>
                    )}
                    <ChevronDown className={cn(
                        "w-4 h-4 text-muted-foreground transition-transform",
                        expanded && "rotate-180"
                    )} />
                </div>
            </button>

            <AnimatePresence>
                {expanded && (
                    <motion.div
                        initial={{ opacity: 0, height: 0 }}
                        animate={{ opacity: 1, height: 'auto' }}
                        exit={{ opacity: 0, height: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="px-5 pb-5 pt-0 border-t border-white/5">
                            <div className="mt-4 grid grid-cols-2 gap-3">
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <Clock className="w-3 h-3" />
                                        Timeout
                                    </div>
                                    <p className="text-sm font-mono font-medium">
                                        {hook.timeout_ms >= 1000
                                            ? `${(hook.timeout_ms / 1000).toFixed(1)}s`
                                            : `${hook.timeout_ms}ms`}
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <Shield className="w-3 h-3" />
                                        Failure Mode
                                    </div>
                                    <p className={cn(
                                        "text-sm font-medium",
                                        isFailClosed ? "text-red-400" : "text-green-400"
                                    )}>
                                        {hook.failure_mode.replace(/([A-Z])/g, ' $1').trim()}
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <ArrowUpDown className="w-3 h-3" />
                                        Priority
                                    </div>
                                    <p className="text-sm font-mono font-medium">
                                        {hook.priority}
                                        <span className="text-muted-foreground/50 text-xs ml-1">
                                            ({hook.priority < 50 ? 'high' : hook.priority < 150 ? 'normal' : 'low'})
                                        </span>
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <Anchor className="w-3 h-3" />
                                        Hook Points
                                    </div>
                                    <p className="text-sm font-mono font-medium">
                                        {hook.hook_points.length}
                                    </p>
                                </div>
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}

function TemplateCard({ template, onActivate, isActive }: { template: HookTemplate; onActivate: (t: HookTemplate) => void; isActive: boolean }) {
    const [showPreview, setShowPreview] = useState(false);
    const [copied, setCopied] = useState(false);
    const [activating, setActivating] = useState(false);

    const handleCopy = (e: React.MouseEvent) => {
        e.stopPropagation();
        navigator.clipboard.writeText(JSON.stringify(template.bundle, null, 2));
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    const handleActivate = async (e: React.MouseEvent) => {
        e.stopPropagation();
        setActivating(true);
        try {
            await onActivate(template);
        } finally {
            setActivating(false);
        }
    };

    return (
        <motion.div
            layout
            className={cn(
                "rounded-xl border transition-all duration-200 group",
                "bg-white/[0.02] border-white/5 hover:border-white/15",
                "hover:shadow-lg hover:shadow-primary/5"
            )}
        >
            <div className="p-4">
                <div className="flex items-start gap-3">
                    <div className={cn("p-2 rounded-lg bg-white/5 border border-border/40", template.color)}>
                        {template.icon}
                    </div>
                    <div className="flex-1 min-w-0">
                        <h4 className="text-sm font-semibold">{template.name}</h4>
                        <p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">{template.description}</p>
                    </div>
                </div>

                <div className="flex items-center gap-2 mt-3">
                    {isActive ? (
                        <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold bg-green-500/10 text-green-400 border border-green-500/20">
                            <Check className="w-3 h-3" />
                            Active
                        </div>
                    ) : (
                        <button
                            onClick={handleActivate}
                            disabled={activating}
                            className={cn(
                                "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold transition-all",
                                "bg-primary/10 hover:bg-primary/20 text-primary border border-primary/20",
                                activating && "opacity-50 cursor-not-allowed"
                            )}
                        >
                            {activating ? (
                                <RefreshCw className="w-3 h-3 animate-spin" />
                            ) : (
                                <Plus className="w-3 h-3" />
                            )}
                            Activate
                        </button>
                    )}
                    <button
                        onClick={(e) => { e.stopPropagation(); setShowPreview(!showPreview); }}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-all bg-white/5 hover:bg-white/10 text-muted-foreground border border-white/5"
                    >
                        <Eye className="w-3 h-3" />
                        Preview
                    </button>
                    <button
                        onClick={handleCopy}
                        className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-xs font-medium transition-all bg-white/5 hover:bg-white/10 text-muted-foreground border border-white/5"
                        title="Copy JSON"
                    >
                        {copied ? <Check className="w-3 h-3 text-green-400" /> : <Copy className="w-3 h-3" />}
                    </button>
                </div>
            </div>

            <AnimatePresence>
                {showPreview && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="px-4 pb-4 border-t border-white/5 pt-3">
                            <pre className="text-[10px] font-mono text-muted-foreground bg-black/30 rounded-lg p-3 overflow-x-auto whitespace-pre-wrap border border-white/5">
                                {JSON.stringify(template.bundle, null, 2)}
                            </pre>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}

// ============================================================================
// Custom Hook Editor Modal
// ============================================================================

function CustomHookModal({ isOpen, onClose, onSubmit }: {
    isOpen: boolean;
    onClose: () => void;
    onSubmit: (json: string) => Promise<void>;
}) {
    const [json, setJson] = useState('{\n  "rules": [\n    {\n      "name": "my-custom-hook",\n      "points": ["beforeOutbound"],\n      "append": "\\n\\n— Custom signature"\n    }\n  ]\n}');
    const [isSubmitting, setIsSubmitting] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const handleSubmit = async () => {
        setError(null);
        try {
            JSON.parse(json);
        } catch (e: any) {
            setError(`Invalid JSON: ${e.message}`);
            return;
        }
        setIsSubmitting(true);
        try {
            await onSubmit(json);
            onClose();
        } catch (e: any) {
            setError(e?.toString() || 'Failed to register hook');
        } finally {
            setIsSubmitting(false);
        }
    };

    if (!isOpen) return null;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
            onClick={onClose}>
            <motion.div
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.95 }}
                className="bg-card border border-border/40 rounded-2xl shadow-2xl w-full max-w-2xl mx-4 overflow-hidden"
                onClick={(e) => e.stopPropagation()}
            >
                <div className="p-6 border-b border-white/5 flex items-center justify-between">
                    <div>
                        <h3 className="text-lg font-bold">Custom Hook</h3>
                        <p className="text-sm text-muted-foreground mt-0.5">
                            Write a custom hook bundle in JSON format.
                        </p>
                    </div>
                    <button onClick={onClose} className="p-2 rounded-lg hover:bg-white/5 text-muted-foreground">
                        <X className="w-4 h-4" />
                    </button>
                </div>
                <div className="p-6">
                    <textarea
                        value={json}
                        onChange={(e) => setJson(e.target.value)}
                        className="w-full h-64 bg-black/30 border border-border/40 rounded-xl p-4 font-mono text-xs text-gray-300 focus:outline-none focus:border-primary/50 resize-none"
                        spellCheck={false}
                    />
                    {error && (
                        <div className="mt-3 p-3 rounded-lg bg-red-500/10 border border-red-500/20 text-red-400 text-xs">
                            {error}
                        </div>
                    )}
                </div>
                <div className="p-6 pt-0 flex justify-end gap-3">
                    <button
                        onClick={onClose}
                        className="px-4 py-2 rounded-lg text-sm font-medium bg-white/5 hover:bg-white/10 transition-colors border border-white/5"
                    >
                        Cancel
                    </button>
                    <button
                        onClick={handleSubmit}
                        disabled={isSubmitting}
                        className={cn(
                            "px-4 py-2 rounded-lg text-sm font-bold transition-all",
                            "bg-primary/20 hover:bg-primary/30 text-primary border border-primary/20",
                            isSubmitting && "opacity-50 cursor-not-allowed"
                        )}
                    >
                        {isSubmitting ? (
                            <RefreshCw className="w-4 h-4 animate-spin inline mr-2" />
                        ) : (
                            <Plus className="w-4 h-4 inline mr-2" />
                        )}
                        Register Hook
                    </button>
                </div>
            </motion.div>
        </div>
    );
}

// ============================================================================
// Main Page
// ============================================================================

export function ThinClawHooks() {
    const [hooks, setHooks] = useState<thinclaw.HookInfoItem[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [showCustomModal, setShowCustomModal] = useState(false);
    const [activeTab, setActiveTab] = useState<'active' | 'templates'>('active');

    const fetchHooks = async () => {
        try {
            const data = await thinclaw.listHooks();
            setHooks(data.hooks || []);
        } catch (e) {
            console.error('Failed to fetch hooks:', e);
            toast.error('Failed to load hooks');
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchHooks();
    }, []);

    const handleActivateTemplate = async (template: HookTemplate) => {
        try {
            const result = await thinclaw.registerHookBundle(
                JSON.stringify(template.bundle),
                `template.${template.id}`
            );
            if (result.ok) {
                toast.success(`Hook "${template.name}" activated`, { description: result.message || undefined });
                await fetchHooks();
                setActiveTab('active');
            } else {
                toast.error('Failed to activate hook', { description: result.message || undefined });
            }
        } catch (e: any) {
            toast.error('Failed to activate hook', { description: e?.toString() });
        }
    };

    const handleRemoveHook = async (hookName: string) => {
        try {
            const result = await thinclaw.unregisterHook(hookName);
            if (result.ok) {
                toast.success(result.message || 'Hook removed');
                await fetchHooks();
            } else {
                toast.warning(result.message || 'Hook not found');
            }
        } catch (e: any) {
            toast.error('Failed to remove hook', { description: e?.toString() });
        }
    };

    const handleCustomSubmit = async (json: string) => {
        const result = await thinclaw.registerHookBundle(json, 'custom');
        if (result.ok) {
            toast.success('Custom hook registered', { description: result.message || undefined });
            await fetchHooks();
            setActiveTab('active');
        } else {
            throw new Error(result.message || 'Registration failed');
        }
    };

    // Group hooks by hook point for the summary
    const hookPointCounts: Record<string, number> = {};
    hooks.forEach(h => {
        h.hook_points.forEach(p => {
            hookPointCounts[p] = (hookPointCounts[p] || 0) + 1;
        });
    });

    // Group templates by category
    const templatesByCategory = HOOK_TEMPLATES.reduce((acc, t) => {
        if (!acc[t.category]) acc[t.category] = [];
        acc[t.category].push(t);
        return acc;
    }, {} as Record<string, HookTemplate[]>);

    // Build set of active hook names for "already activated" detection
    const activeHookNames = new Set(hooks.map(h => h.name));

    // Check if a template has any of its rules already active
    const isTemplateActive = (template: HookTemplate): boolean => {
        const bundle = template.bundle as any;
        if (!bundle?.rules) return false;
        return bundle.rules.some((rule: any) =>
            Array.from(activeHookNames).some(name => name.includes(`::${rule.name}`))
        );
    };

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 flex flex-col h-full overflow-hidden"
        >
            <div className="p-8 pb-4 space-y-6 flex-none max-w-5xl w-full mx-auto">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                    <div>
                        <h1 className="text-3xl font-bold tracking-tight">Lifecycle Hooks</h1>
                        <p className="text-muted-foreground mt-1">
                            Middleware for your agent — filter, transform, or reject events at any lifecycle point.
                        </p>
                    </div>

                    <div className="flex items-center gap-3">
                        <button
                            onClick={() => setShowCustomModal(true)}
                            className="px-4 py-2 rounded-xl bg-white/5 border border-border/40 hover:bg-white/10 transition-colors text-sm font-medium flex items-center gap-2"
                        >
                            <Code2 className="w-4 h-4" />
                            Custom Hook
                        </button>
                        <div className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary flex items-center gap-2 text-sm font-bold shadow-lg shadow-primary/5">
                            <Anchor className="w-4 h-4" />
                            {hooks.length} active
                        </div>
                        <button
                            onClick={() => {
                                setIsLoading(true);
                                fetchHooks();
                            }}
                            className="p-2.5 rounded-xl bg-card border border-border/40 hover:bg-white/5 transition-colors shadow-sm"
                        >
                            <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                        </button>
                    </div>
                </div>

                {/* Tab Switcher */}
                <div className="flex gap-1 bg-white/[0.03] border border-white/5 rounded-xl p-1">
                    <button
                        onClick={() => setActiveTab('active')}
                        className={cn(
                            "flex-1 px-4 py-2 rounded-lg text-sm font-medium transition-all",
                            activeTab === 'active'
                                ? "bg-white/10 text-white shadow-sm"
                                : "text-muted-foreground hover:text-white hover:bg-white/5"
                        )}
                    >
                        <Anchor className="w-3.5 h-3.5 inline mr-2" />
                        Active Hooks ({hooks.length})
                    </button>
                    <button
                        onClick={() => setActiveTab('templates')}
                        className={cn(
                            "flex-1 px-4 py-2 rounded-lg text-sm font-medium transition-all",
                            activeTab === 'templates'
                                ? "bg-white/10 text-white shadow-sm"
                                : "text-muted-foreground hover:text-white hover:bg-white/5"
                        )}
                    >
                        <Sparkles className="w-3.5 h-3.5 inline mr-2" />
                        Hook Templates ({HOOK_TEMPLATES.length})
                    </button>
                </div>

                {/* Hook point summary (only on active tab) */}
                {activeTab === 'active' && Object.keys(hookPointCounts).length > 0 && (
                    <div className="flex flex-wrap gap-2">
                        {Object.entries(hookPointCounts)
                            .sort(([, a], [, b]) => b - a)
                            .map(([point, count]) => (
                                <div
                                    key={point}
                                    className={cn(
                                        "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium border",
                                        HOOK_POINT_STYLE
                                    )}
                                >
                                    {HOOK_POINT_ICONS[point]}
                                    {point}
                                    <span className="font-bold ml-0.5">× {count}</span>
                                </div>
                            ))}
                    </div>
                )}
            </div>

            <div className="flex-1 overflow-y-auto px-8 pb-8 scrollbar-hide">
                <div className="max-w-5xl mx-auto space-y-3">
                    {/* Active Hooks Tab */}
                    {activeTab === 'active' && (
                        <>
                            {isLoading && hooks.length === 0 ? (
                                <div className="space-y-3">
                                    {[1, 2, 3].map(i => (
                                        <div key={i} className="h-24 rounded-2xl border border-white/5 bg-white/[0.02] animate-pulse" />
                                    ))}
                                </div>
                            ) : hooks.length > 0 ? (
                                <AnimatePresence mode="popLayout">
                                    {hooks.map(hook => (
                                        <HookCard
                                            key={hook.name}
                                            hook={hook}
                                            onRemove={() => handleRemoveHook(hook.name)}
                                        />
                                    ))}
                                </AnimatePresence>
                            ) : (
                                <div className="py-16 flex flex-col items-center justify-center text-center space-y-4">
                                    <div className="p-4 rounded-full bg-white/5 border border-border/40">
                                        <Anchor className="w-8 h-8 text-muted-foreground" />
                                    </div>
                                    <div>
                                        <h3 className="text-lg font-semibold">No active hooks</h3>
                                        <p className="text-sm text-muted-foreground mt-1 max-w-md">
                                            Hooks are middleware that intercept events in the agent pipeline. Browse the{' '}
                                            <button onClick={() => setActiveTab('templates')} className="text-primary hover:underline font-medium">
                                                template gallery
                                            </button>{' '}
                                            to get started, or create a custom hook.
                                        </p>
                                    </div>
                                    <div className="flex gap-3 mt-2">
                                        <button
                                            onClick={() => setActiveTab('templates')}
                                            className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary text-sm font-bold hover:bg-primary/20 transition-colors flex items-center gap-2"
                                        >
                                            <Sparkles className="w-4 h-4" />
                                            Browse Templates
                                        </button>
                                        <button
                                            onClick={() => setShowCustomModal(true)}
                                            className="px-4 py-2 rounded-xl bg-white/5 border border-border/40 text-sm font-medium hover:bg-white/10 transition-colors flex items-center gap-2"
                                        >
                                            <Code2 className="w-4 h-4" />
                                            Write Custom
                                        </button>
                                    </div>
                                </div>
                            )}

                            {/* Info section */}
                            <div className="mt-8 p-6 rounded-2xl border bg-primary/5 border-primary/10 flex gap-4">
                                <div className="p-2 bg-primary/10 rounded-xl h-fit">
                                    <AlertCircle className="w-5 h-5 text-primary" />
                                </div>
                                <div>
                                    <h4 className="text-sm font-semibold text-primary uppercase tracking-wider">Hook Pipeline</h4>
                                    <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                                        Hooks execute in priority order (lower number = runs first). A hook can pass through,
                                        modify content, or reject the event entirely. <strong>Fail-Open</strong> hooks continue on error,
                                        while <strong>Fail-Closed</strong> hooks block the pipeline. Active hooks persist until the engine restarts.
                                    </p>
                                </div>
                            </div>
                        </>
                    )}

                    {/* Templates Tab */}
                    {activeTab === 'templates' && (
                        <div className="space-y-8">
                            {Object.entries(templatesByCategory).map(([category, templates]) => (
                                <div key={category}>
                                    <h3 className="text-sm font-bold uppercase tracking-wider mb-3 flex items-center gap-2 text-muted-foreground">
                                        {CATEGORY_LABELS[category]?.label || category}
                                    </h3>
                                    <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                                        {templates.map(template => (
                                            <TemplateCard
                                                key={template.id}
                                                template={template}
                                                onActivate={handleActivateTemplate}
                                                isActive={isTemplateActive(template)}
                                            />
                                        ))}
                                    </div>
                                </div>
                            ))}

                            {/* Custom hook CTA */}
                            <div className="p-6 rounded-2xl border border-dashed border-border/40 bg-white/[0.01] flex items-center justify-between">
                                <div>
                                    <h4 className="text-sm font-semibold">Need something custom?</h4>
                                    <p className="text-xs text-muted-foreground mt-0.5">
                                        Write your own hook bundle with regex rules, content transforms, or outbound webhooks.
                                    </p>
                                </div>
                                <button
                                    onClick={() => setShowCustomModal(true)}
                                    className="px-4 py-2 rounded-xl bg-white/5 border border-border/40 text-sm font-medium hover:bg-white/10 transition-colors flex items-center gap-2 flex-none"
                                >
                                    <Code2 className="w-4 h-4" />
                                    Write Custom Hook
                                </button>
                            </div>
                        </div>
                    )}
                </div>
            </div>

            <CustomHookModal
                isOpen={showCustomModal}
                onClose={() => setShowCustomModal(false)}
                onSubmit={handleCustomSubmit}
            />
        </motion.div>
    );
}
