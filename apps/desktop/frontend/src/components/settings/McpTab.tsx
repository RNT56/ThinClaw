import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import * as Switch from '@radix-ui/react-switch';
import {
    Plug,
    Server,
    CheckCircle2,
    XCircle,
    AlertTriangle,
    Loader2,
    Globe,
    Zap,
    ShieldCheck,
    Clock,
    Hash,
    RefreshCw,
    Eye,
    EyeOff,
    Info,
    Sparkles,
    Link,
} from 'lucide-react';
import { commands } from '../../lib/bindings';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';

type ConnectionStatus = 'idle' | 'testing' | 'connected' | 'error';

interface ConnectionResult {
    status: ConnectionStatus;
    message: string;
    toolCount?: number;
    latencyMs?: number;
}

// ─── Reusable Field Components ────────────────────────────────────────────────

function SettingRow({
    label,
    description,
    badge,
    children,
}: {
    label: string;
    description?: string;
    badge?: React.ReactNode;
    children: React.ReactNode;
}) {
    return (
        <div className="flex items-start justify-between gap-8 py-5 border-b border-border/30 last:border-b-0">
            <div className="space-y-1 flex-1 min-w-0">
                <div className="flex items-center gap-2">
                    <label className="text-sm font-semibold">{label}</label>
                    {badge}
                </div>
                {description && (
                    <p className="text-xs text-muted-foreground leading-relaxed">{description}</p>
                )}
            </div>
            <div className="shrink-0">{children}</div>
        </div>
    );
}

function SectionCard({
    title,
    icon: Icon,
    iconColor = 'text-primary',
    iconBg = 'bg-primary/10',
    children,
}: {
    title: string;
    icon: React.ElementType;
    iconColor?: string;
    iconBg?: string;
    children: React.ReactNode;
}) {
    return (
        <div className="p-6 border border-border/50 rounded-xl bg-card shadow-sm space-y-0">
            <div className="flex items-center gap-3 mb-4 pb-4 border-b border-border/30">
                <div className={cn('p-2 rounded-lg', iconBg)}>
                    <Icon className={cn('w-4 h-4', iconColor)} />
                </div>
                <h3 className="font-semibold text-base">{title}</h3>
            </div>
            {children}
        </div>
    );
}

function TextInput({
    value,
    onChange,
    onBlur,
    placeholder,
    type = 'text',
    disabled,
}: {
    value: string;
    onChange: (v: string) => void;
    onBlur?: () => void;
    placeholder?: string;
    type?: string;
    disabled?: boolean;
}) {
    return (
        <input
            type={type}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onBlur={onBlur}
            placeholder={placeholder}
            disabled={disabled}
            spellCheck={false}
            autoComplete="off"
            className={cn(
                'w-[320px] h-10 px-3 rounded-xl border bg-background/70 text-sm font-mono transition-all duration-200 backdrop-blur-sm outline-none',
                'border-border/50 focus:border-primary focus:ring-2 focus:ring-primary/20',
                disabled && 'opacity-50 cursor-not-allowed'
            )}
        />
    );
}

function ToggleSwitch({
    checked,
    onCheckedChange,
    color = 'bg-primary',
}: {
    checked: boolean;
    onCheckedChange: (v: boolean) => void;
    color?: string;
}) {
    return (
        <Switch.Root
            checked={checked}
            onCheckedChange={onCheckedChange}
            className={cn(
                'w-[42px] h-[25px] rounded-full relative shadow-inner transition-colors duration-200 cursor-pointer outline-none',
                checked ? color : 'bg-muted'
            )}
        >
            <Switch.Thumb className="block w-[21px] h-[21px] bg-white rounded-full shadow-[0_2px_4px_rgba(0,0,0,0.25)] transition-transform duration-150 translate-x-0.5 will-change-transform data-[state=checked]:translate-x-[19px]" />
        </Switch.Root>
    );
}

// ─── Connection Status Pill ───────────────────────────────────────────────────

function StatusPill({ result }: { result: ConnectionResult }) {
    const variants = {
        idle: {
            bg: 'bg-muted/50',
            text: 'text-muted-foreground',
            border: 'border-border/40',
            dot: 'bg-muted-foreground/40',
            icon: null,
        },
        testing: {
            bg: 'bg-blue-500/10',
            text: 'text-blue-400',
            border: 'border-blue-500/20',
            dot: 'bg-blue-400 animate-pulse',
            icon: <Loader2 className="w-3 h-3 animate-spin" />,
        },
        connected: {
            bg: 'bg-emerald-500/10',
            text: 'text-emerald-400',
            border: 'border-emerald-500/20',
            dot: 'bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,0.6)]',
            icon: <CheckCircle2 className="w-3 h-3" />,
        },
        error: {
            bg: 'bg-rose-500/10',
            text: 'text-rose-400',
            border: 'border-rose-500/20',
            dot: 'bg-rose-400 shadow-[0_0_8px_rgba(251,113,133,0.5)]',
            icon: <XCircle className="w-3 h-3" />,
        },
    }[result.status];

    return (
        <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            className={cn(
                'inline-flex items-center gap-2 px-3 py-1.5 rounded-full border text-xs font-medium',
                variants.bg, variants.text, variants.border
            )}
        >
            {variants.icon ? variants.icon : <div className={cn('w-2 h-2 rounded-full', variants.dot)} />}
            <span>{result.message}</span>
            {result.toolCount !== undefined && result.status === 'connected' && (
                <span className="opacity-60 border-l border-current/20 pl-2 flex items-center gap-1">
                    <Hash className="w-2.5 h-2.5" />
                    {result.toolCount} tools
                </span>
            )}
            {result.latencyMs !== undefined && result.status === 'connected' && (
                <span className="opacity-60 border-l border-current/20 pl-2 flex items-center gap-1">
                    <Clock className="w-2.5 h-2.5" />
                    {result.latencyMs}ms
                </span>
            )}
        </motion.div>
    );
}

// ─── Tool List Preview ────────────────────────────────────────────────────────

function ToolChip({ name }: { name: string }) {
    return (
        <motion.div
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-primary/8 border border-primary/15 text-xs font-mono text-primary/80"
        >
            <Sparkles className="w-2.5 h-2.5 opacity-60" />
            {name}
        </motion.div>
    );
}

// ─── Main McpTab ──────────────────────────────────────────────────────────────

export function McpTab() {
    const [config, setConfig] = useState<any>(null);
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);

    // Local draft state (applied on blur/save)
    const [baseUrl, setBaseUrl] = useState('');
    const [authToken, setAuthToken] = useState('');
    const [showToken, setShowToken] = useState(false);
    const [sandboxEnabled, setSandboxEnabled] = useState(false);
    const [cacheTtl, setCacheTtl] = useState(300);
    const [maxResultChars, setMaxResultChars] = useState(5000);

    const [connectionResult, setConnectionResult] = useState<ConnectionResult>({
        status: 'idle',
        message: 'Not tested',
    });
    const [discoveredTools, setDiscoveredTools] = useState<string[]>([]);

    // ── Load config ──
    useEffect(() => {
        commands.getUserConfig().then((cfg: any) => {
            if (cfg) {
                setConfig(cfg);
                setBaseUrl(cfg.mcp_base_url ?? '');
                setAuthToken(cfg.mcp_auth_token ?? '');
                setSandboxEnabled(cfg.mcp_sandbox_enabled ?? false);
                setCacheTtl(cfg.mcp_cache_ttl_secs ?? 300);
                setMaxResultChars(cfg.mcp_tool_result_max_chars ?? 5000);
            }
            setLoading(false);
        });
    }, []);

    // ── Persist helpers ──
    const persist = useCallback(async (patch: object) => {
        if (!config) return;
        setSaving(true);
        try {
            const next = { ...config, ...patch };
            setConfig(next);
            await commands.updateUserConfig(next);
        } catch (e) {
            toast.error('Failed to save MCP settings', { description: String(e) });
        } finally {
            setSaving(false);
        }
    }, [config]);

    const handleUrlBlur = () => persist({ mcp_base_url: baseUrl || null });
    const handleTokenBlur = () => persist({ mcp_auth_token: authToken || null });
    const handleCacheTtlBlur = () => persist({ mcp_cache_ttl_secs: cacheTtl });
    const handleMaxCharsBlur = () => persist({ mcp_tool_result_max_chars: maxResultChars });

    // ── Test Connection ──
    const testConnection = async () => {
        if (!baseUrl) {
            toast.error('Enter a server URL first.');
            return;
        }
        setConnectionResult({ status: 'testing', message: 'Connecting…' });
        setDiscoveredTools([]);

        const start = Date.now();
        try {
            // Use the rig_check_web_search as a proxy — or call the MCP tool list endpoint
            // We call the backend's checkWebSearch which exercises the search pipeline
            const toolListUrl = baseUrl.replace(/\/$/, '') + '/tools';
            const headers: Record<string, string> = { 'Content-Type': 'application/json' };
            if (authToken) headers['Authorization'] = `Bearer ${authToken}`;

            const res = await fetch(toolListUrl, { headers, signal: AbortSignal.timeout(8000) });
            const latencyMs = Date.now() - start;

            if (!res.ok) {
                setConnectionResult({
                    status: 'error',
                    message: `Server returned ${res.status} ${res.statusText}`,
                });
                return;
            }

            const data = await res.json();
            const tools: string[] = Array.isArray(data)
                ? data.map((t: any) => t.name ?? t.id ?? String(t))
                : Object.keys(data?.tools ?? data ?? {});

            setDiscoveredTools(tools);
            setConnectionResult({
                status: 'connected',
                message: 'Connected',
                toolCount: tools.length,
                latencyMs,
            });
            toast.success('MCP server reachable', {
                description: `${tools.length} tool${tools.length !== 1 ? 's' : ''} discovered in ${latencyMs}ms`,
            });
        } catch (err: any) {
            const msg = err?.name === 'TimeoutError'
                ? 'Request timed out (8s)'
                : err?.message ?? 'Connection refused';
            setConnectionResult({ status: 'error', message: msg });
            toast.error('Connection failed', { description: msg });
        }
    };

    if (loading) {
        return (
            <div className="space-y-4 animate-pulse pt-2">
                {[1, 2, 3].map(i => (
                    <div key={i} className="h-24 rounded-xl bg-muted/40 border border-border/30" />
                ))}
            </div>
        );
    }

    const hasMcpUrl = baseUrl.trim().length > 0;
    const isConfigured = hasMcpUrl && sandboxEnabled;

    return (
        <div className="space-y-6">

            {/* ── Status Banner ── */}
            <AnimatePresence>
                {isConfigured && (
                    <motion.div
                        initial={{ opacity: 0, y: -8, height: 0 }}
                        animate={{ opacity: 1, y: 0, height: 'auto' }}
                        exit={{ opacity: 0, y: -8, height: 0 }}
                        className="flex items-center gap-3 px-4 py-3 rounded-xl bg-emerald-500/8 border border-emerald-500/20 text-emerald-400"
                    >
                        <div className="relative">
                            <div className="absolute inset-0 bg-emerald-500/20 rounded-full animate-ping opacity-40" />
                            <ShieldCheck className="relative w-4 h-4" />
                        </div>
                        <div className="text-sm">
                            <span className="font-semibold">MCP Sandbox Active</span>
                            <span className="opacity-70 ml-2 font-normal">
                                — Tools and remote skills are enabled for the AI agent
                            </span>
                        </div>
                    </motion.div>
                )}
                {!isConfigured && hasMcpUrl && !sandboxEnabled && (
                    <motion.div
                        initial={{ opacity: 0, y: -8, height: 0 }}
                        animate={{ opacity: 1, y: 0, height: 'auto' }}
                        exit={{ opacity: 0, y: -8, height: 0 }}
                        className="flex items-center gap-3 px-4 py-3 rounded-xl bg-amber-500/8 border border-amber-500/20 text-amber-400"
                    >
                        <AlertTriangle className="w-4 h-4 shrink-0" />
                        <p className="text-sm">
                            URL is set but the <span className="font-semibold">Sandbox is disabled</span>. Enable it below to activate remote tool execution.
                        </p>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* ── Server URL ── */}
            <SectionCard title="Server Connection" icon={Globe} iconColor="text-blue-400" iconBg="bg-blue-500/10">
                <SettingRow
                    label="Server Base URL"
                    description="The base URL of your FastAPI MCP server. All tool routes are resolved relative to this endpoint."
                    badge={
                        hasMcpUrl
                            ? <span className="text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded bg-primary/10 text-primary">Active</span>
                            : null
                    }
                >
                    <div className="flex flex-col items-end gap-2">
                        <TextInput
                            value={baseUrl}
                            onChange={setBaseUrl}
                            onBlur={handleUrlBlur}
                            placeholder="https://api.yourserver.com"
                        />
                        <p className="text-[10px] text-muted-foreground">
                            e.g. <span className="font-mono text-primary/70">http://localhost:8000</span> for local dev
                        </p>
                    </div>
                </SettingRow>

                <SettingRow
                    label="Auth Token"
                    description="JWT bearer token sent as Authorization header with every MCP request. Leave empty for unauthenticated servers."
                >
                    <div className="relative w-[320px]">
                        <input
                            type={showToken ? 'text' : 'password'}
                            value={authToken}
                            onChange={(e) => setAuthToken(e.target.value)}
                            onBlur={handleTokenBlur}
                            placeholder="••••••••••••••••••••••••"
                            spellCheck={false}
                            autoComplete="new-password"
                            className="w-full h-10 pl-3 pr-10 rounded-xl border border-border/50 bg-background/70 text-sm font-mono transition-all duration-200 backdrop-blur-sm outline-none focus:border-primary focus:ring-2 focus:ring-primary/20"
                        />
                        <button
                            type="button"
                            onClick={() => setShowToken(v => !v)}
                            className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors"
                        >
                            {showToken
                                ? <EyeOff className="w-3.5 h-3.5" />
                                : <Eye className="w-3.5 h-3.5" />
                            }
                        </button>
                    </div>
                </SettingRow>

                {/* Test Connection */}
                <div className="pt-4 flex items-center justify-between">
                    <div className="flex items-center gap-3">
                        <StatusPill result={connectionResult} />
                    </div>
                    <button
                        onClick={testConnection}
                        disabled={!hasMcpUrl || connectionResult.status === 'testing'}
                        className={cn(
                            'inline-flex items-center gap-2 h-9 px-4 rounded-xl text-sm font-medium border transition-all duration-200',
                            hasMcpUrl && connectionResult.status !== 'testing'
                                ? 'bg-primary/10 border-primary/30 text-primary hover:bg-primary/20 hover:border-primary/50 shadow-sm hover:shadow-primary/10'
                                : 'bg-muted/40 border-border/30 text-muted-foreground cursor-not-allowed opacity-50'
                        )}
                    >
                        {connectionResult.status === 'testing'
                            ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                            : <Plug className="w-3.5 h-3.5" />
                        }
                        Test Connection
                    </button>
                </div>

                {/* Discovered Tools */}
                <AnimatePresence>
                    {discoveredTools.length > 0 && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: 'auto' }}
                            exit={{ opacity: 0, height: 0 }}
                            className="overflow-hidden"
                        >
                            <div className="mt-4 pt-4 border-t border-border/30">
                                <div className="flex items-center gap-2 mb-3">
                                    <Zap className="w-3.5 h-3.5 text-primary opacity-60" />
                                    <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                                        Discovered Tools
                                    </span>
                                    <div className="h-px flex-1 bg-border/30" />
                                    <span className="text-[10px] text-muted-foreground bg-secondary/50 px-2 py-0.5 rounded-full">
                                        {discoveredTools.length} total
                                    </span>
                                </div>
                                <div className="flex flex-wrap gap-2">
                                    {discoveredTools.map(name => (
                                        <ToolChip key={name} name={name} />
                                    ))}
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </SectionCard>

            {/* ── Sandbox Control ── */}
            <SectionCard title="Sandbox Execution" icon={Server} iconColor="text-violet-400" iconBg="bg-violet-500/10">
                <SettingRow
                    label="Enable MCP Sandbox"
                    description="Allows the AI agent to discover and execute tools on your MCP server via Rhai scripts during conversations. Requires a valid Server URL above."
                >
                    <ToggleSwitch
                        checked={sandboxEnabled}
                        color="bg-violet-500"
                        onCheckedChange={async (val) => {
                            if (val && !hasMcpUrl) {
                                toast.error('Set a Server URL first before enabling the sandbox.');
                                return;
                            }
                            setSandboxEnabled(val);
                            await persist({ mcp_sandbox_enabled: val });
                            if (val) {
                                toast.success('MCP Sandbox enabled', {
                                    description: 'The AI agent can now invoke remote tools.',
                                });
                            }
                        }}
                    />
                </SettingRow>

                <AnimatePresence>
                    {sandboxEnabled && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: 'auto' }}
                            exit={{ opacity: 0, height: 0 }}
                            className="overflow-hidden"
                        >
                            <div className="mt-2 p-3 rounded-lg bg-violet-500/5 border border-violet-500/15 text-xs text-violet-300/80 leading-relaxed flex gap-2">
                                <Info className="w-3.5 h-3.5 shrink-0 mt-0.5 opacity-70" />
                                <span>
                                    With the sandbox active, the LLM can call <code className="font-mono bg-violet-500/10 px-1 rounded">mcp_call(tool, args)</code> and <code className="font-mono bg-violet-500/10 px-1 rounded">search_tools(query)</code> during each chat turn. Tool results are injected back into the conversation context.
                                </span>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </SectionCard>

            {/* ── Performance Tuning ── */}
            <SectionCard title="Performance & Caching" icon={RefreshCw} iconColor="text-amber-400" iconBg="bg-amber-500/10">
                <SettingRow
                    label="Tool Cache TTL"
                    description="How long (in seconds) tool discovery results are cached before re-fetching from the server. Lower values are more accurate but increase network calls."
                    badge={
                        <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-muted text-muted-foreground">
                            {cacheTtl}s
                        </span>
                    }
                >
                    <div className="flex items-center gap-3">
                        <input
                            type="range"
                            min="30"
                            max="3600"
                            step="30"
                            value={cacheTtl}
                            onChange={(e) => setCacheTtl(Number(e.target.value))}
                            onMouseUp={handleCacheTtlBlur}
                            onTouchEnd={handleCacheTtlBlur}
                            className="w-[180px] h-2 bg-muted rounded-lg appearance-none cursor-pointer accent-amber-500"
                        />
                        <span className="text-sm font-bold w-16 text-right tabular-nums">
                            {cacheTtl >= 60
                                ? `${(cacheTtl / 60).toFixed(0)}m`
                                : `${cacheTtl}s`
                            }
                        </span>
                    </div>
                </SettingRow>

                <SettingRow
                    label="Max Tool Result Size"
                    description="Maximum characters of a tool's response injected into the LLM context. Larger values give the model more detail but consume more tokens."
                    badge={
                        <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-muted text-muted-foreground">
                            {maxResultChars.toLocaleString()} chars
                        </span>
                    }
                >
                    <div className="flex items-center gap-3">
                        <input
                            type="range"
                            min="1000"
                            max="50000"
                            step="1000"
                            value={maxResultChars}
                            onChange={(e) => setMaxResultChars(Number(e.target.value))}
                            onMouseUp={handleMaxCharsBlur}
                            onTouchEnd={handleMaxCharsBlur}
                            className="w-[180px] h-2 bg-muted rounded-lg appearance-none cursor-pointer accent-amber-500"
                        />
                        <span className="text-sm font-bold w-16 text-right tabular-nums">
                            {maxResultChars >= 1000
                                ? `${(maxResultChars / 1000).toFixed(0)}k`
                                : maxResultChars
                            }
                        </span>
                    </div>
                </SettingRow>
            </SectionCard>

            {/* ── Environment Variable Hint ── */}
            <div className="p-4 rounded-xl bg-muted/20 border border-border/30 space-y-2">
                <div className="flex items-center gap-2 text-xs font-semibold text-muted-foreground uppercase tracking-wider">
                    <Link className="w-3 h-3" />
                    Environment Variable Overrides
                </div>
                <p className="text-xs text-muted-foreground leading-relaxed">
                    Settings here can also be provided via environment variables before app launch, useful for CI or managed deployments:
                </p>
                <div className="grid grid-cols-2 gap-2 mt-1">
                    {[
                        ['THINCLAW_MCP_URL', 'Server Base URL'],
                        ['THINCLAW_MCP_TOKEN', 'Auth Token'],
                    ].map(([env, label]) => (
                        <div key={env} className="flex flex-col gap-0.5 bg-background/50 p-2.5 rounded-lg border border-border/30">
                            <code className="text-[11px] font-mono text-primary/80">{env}</code>
                            <span className="text-[10px] text-muted-foreground/70">{label}</span>
                        </div>
                    ))}
                </div>
                <p className="text-[10px] text-muted-foreground/60 italic">
                    Values set in config above take precedence over environment variables.
                </p>
            </div>

            {/* Saving indicator */}
            <AnimatePresence>
                {saving && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="fixed bottom-6 right-6 flex items-center gap-2 px-3 py-2 rounded-xl bg-card border border-border shadow-xl text-xs text-muted-foreground"
                    >
                        <Loader2 className="w-3 h-3 animate-spin" />
                        Saving…
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
