import type { ReactNode } from 'react';
import {
    AlertCircle, Anchor, ArrowUpDown, Ban, ChevronRight, Code2, FileText, Filter, Globe,
    Languages, Lock, RefreshCw, Shield, ShieldAlert, Sparkles, Zap,
} from 'lucide-react';

// Uniform hook point pill style — matches the app's neutral theme
export const HOOK_POINT_STYLE = 'bg-white/5 text-muted-foreground border-border/40';

export const HOOK_POINT_ICONS: Record<string, ReactNode> = {
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

export interface HookTemplate {
    id: string;
    name: string;
    description: string;
    category: 'safety' | 'workflow' | 'security' | 'formatting';
    icon: React.ReactNode;
    color: string;
    bundle: object;
}

export const HOOK_TEMPLATES: HookTemplate[] = [
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

export const CATEGORY_LABELS: Record<string, { label: string }> = {
    safety: { label: 'Safety & Privacy' },
    workflow: { label: 'Workflow & Productivity' },
    security: { label: 'Security & Control' },
    formatting: { label: 'Response Formatting' },
};
