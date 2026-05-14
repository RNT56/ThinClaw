import { useState, useEffect, useCallback, useRef } from 'react';
import { motion, AnimatePresence, Reorder } from 'framer-motion';
import {
    GitBranch, RefreshCw, Zap, Info, Cpu, Layers,
    ArrowRight, CheckCircle2, Plus, Trash2, GripVertical,
    Save, ChevronDown, ChevronUp, Tag, Hash, Globe,
    Sparkles, AlertCircle, PenLine
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

type MatchKind = 'keyword' | 'context_length' | 'provider' | 'always';

const MATCH_KIND_OPTIONS: { value: MatchKind; label: string; icon: any; description: string }[] = [
    { value: 'keyword', label: 'Keyword Match', icon: Tag, description: 'Route when prompt contains specific keywords' },
    { value: 'context_length', label: 'Context Length', icon: Hash, description: 'Route when context exceeds a token threshold' },
    { value: 'provider', label: 'Provider Preference', icon: Globe, description: 'Route all requests for a specific provider' },
    { value: 'always', label: 'Default Fallback', icon: Sparkles, description: 'Catch-all rule — lowest priority' },
];

const MATCH_KIND_COLORS: Record<MatchKind, string> = {
    keyword: 'text-blue-400 bg-blue-500/10 border-blue-500/20',
    context_length: 'text-muted-foreground bg-amber-500/10 border-amber-500/20',
    provider: 'text-primary bg-emerald-500/10 border-emerald-500/20',
    always: 'text-primary bg-violet-500/10 border-violet-500/20',
};

function generateId(): string {
    return crypto.randomUUID?.() ?? `rule-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function createEmptyRule(priority: number): openclaw.RoutingRule {
    return {
        id: generateId(),
        label: '',
        match_kind: 'keyword',
        match_value: '',
        target_model: '',
        target_provider: null,
        priority,
        enabled: true,
    };
}

// ── Rule Card Component ──────────────────────────────────────────────

interface RuleCardProps {
    rule: openclaw.RoutingRule;
    index: number;
    onUpdate: (id: string, patch: Partial<openclaw.RoutingRule>) => void;
    onDelete: (id: string) => void;
    isExpanded: boolean;
    onToggleExpand: (id: string) => void;
}

function RuleCard({ rule, index, onUpdate, onDelete, isExpanded, onToggleExpand }: RuleCardProps) {
    const matchMeta = MATCH_KIND_OPTIONS.find(m => m.value === rule.match_kind) ?? MATCH_KIND_OPTIONS[0];
    const MatchIcon = matchMeta.icon;
    const colorClass = MATCH_KIND_COLORS[rule.match_kind] || MATCH_KIND_COLORS.keyword;

    return (
        <Reorder.Item
            value={rule}
            id={rule.id}
            className="group"
            dragListener={false}
        >
            <motion.div
                layout
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -8, scale: 0.95 }}
                transition={{ duration: 0.2 }}
                className={cn(
                    "rounded-xl border bg-card/30 backdrop-blur-md overflow-hidden transition-all duration-200",
                    rule.enabled
                        ? "border-border/40 hover:border-white/20"
                        : "border-white/5 opacity-60"
                )}
            >
                {/* Header Row */}
                <div className="flex items-center gap-3 p-4">
                    {/* Drag handle */}
                    <div className="cursor-grab active:cursor-grabbing touch-none opacity-30 group-hover:opacity-60 transition-opacity">
                        <GripVertical className="w-4 h-4" />
                    </div>

                    {/* Priority badge */}
                    <div className="flex items-center justify-center w-6 h-6 rounded-md bg-white/5 border border-border/40 text-[10px] font-bold text-muted-foreground shrink-0">
                        {index + 1}
                    </div>

                    {/* Match type icon */}
                    <div className={cn("p-1.5 rounded-lg border shrink-0", colorClass)}>
                        <MatchIcon className="w-3.5 h-3.5" />
                    </div>

                    {/* Label / summary */}
                    <div className="flex-1 min-w-0">
                        <div className="text-sm font-medium truncate">
                            {rule.label || <span className="text-muted-foreground italic">Untitled rule</span>}
                        </div>
                        <div className="text-[11px] text-muted-foreground mt-0.5 truncate">
                            {matchMeta.label}
                            {rule.match_value && rule.match_kind !== 'always' && (
                                <> · <span className="text-foreground/60">{rule.match_value}</span></>
                            )}
                            {rule.target_model && (
                                <> → <span className="text-foreground/80 font-medium">{rule.target_model}</span></>
                            )}
                        </div>
                    </div>

                    {/* Enable toggle */}
                    <button
                        onClick={() => onUpdate(rule.id, { enabled: !rule.enabled })}
                        className={cn(
                            "relative w-9 h-5 rounded-full transition-all duration-300 shrink-0",
                            rule.enabled ? "bg-emerald-500" : "bg-zinc-700"
                        )}
                    >
                        <motion.div
                            className="absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow-md"
                            animate={{ x: rule.enabled ? 16 : 0 }}
                            transition={{ type: "spring", stiffness: 500, damping: 30 }}
                        />
                    </button>

                    {/* Expand toggle */}
                    <button
                        onClick={() => onToggleExpand(rule.id)}
                        className="p-1.5 rounded-lg hover:bg-white/5 text-muted-foreground transition-colors"
                    >
                        {isExpanded ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                    </button>
                </div>

                {/* Expanded Editor */}
                <AnimatePresence>
                    {isExpanded && (
                        <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: 'auto', opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            transition={{ duration: 0.25, ease: 'easeInOut' }}
                            className="overflow-hidden"
                        >
                            <div className="border-t border-white/5 p-4 space-y-4">
                                {/* Row 1: Label */}
                                <div className="space-y-1.5">
                                    <label className="text-[11px] font-medium uppercase tracking-widest text-muted-foreground/60 flex items-center gap-1.5">
                                        <PenLine className="w-3 h-3" /> Label
                                    </label>
                                    <input
                                        type="text"
                                        value={rule.label}
                                        onChange={e => onUpdate(rule.id, { label: e.target.value })}
                                        placeholder='e.g. "Code tasks → GPT-4o"'
                                        className="w-full px-3 py-2 rounded-lg bg-white/5 border border-border/40 text-sm focus:outline-none focus:ring-1 focus:ring-violet-500/50 placeholder:text-muted-foreground/40"
                                    />
                                </div>

                                {/* Row 2: Match kind + value */}
                                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                                    <div className="space-y-1.5">
                                        <label className="text-[11px] font-medium uppercase tracking-widest text-muted-foreground/60">
                                            Match Condition
                                        </label>
                                        <select
                                            value={rule.match_kind}
                                            onChange={e => onUpdate(rule.id, {
                                                match_kind: e.target.value as MatchKind,
                                                match_value: e.target.value === 'always' ? '' : rule.match_value
                                            })}
                                            className="w-full px-3 py-2 rounded-lg bg-white/5 border border-border/40 text-sm focus:outline-none focus:ring-1 focus:ring-violet-500/50 appearance-none cursor-pointer"
                                        >
                                            {MATCH_KIND_OPTIONS.map(opt => (
                                                <option key={opt.value} value={opt.value}>{opt.label}</option>
                                            ))}
                                        </select>
                                        <p className="text-[10px] text-muted-foreground/50 mt-1">{matchMeta.description}</p>
                                    </div>

                                    {rule.match_kind !== 'always' && (
                                        <div className="space-y-1.5">
                                            <label className="text-[11px] font-medium uppercase tracking-widest text-muted-foreground/60">
                                                {rule.match_kind === 'keyword' ? 'Keywords (comma-separated)' :
                                                    rule.match_kind === 'context_length' ? 'Token Threshold' :
                                                        'Provider Name'}
                                            </label>
                                            <input
                                                type={rule.match_kind === 'context_length' ? 'number' : 'text'}
                                                value={rule.match_value}
                                                onChange={e => onUpdate(rule.id, { match_value: e.target.value })}
                                                placeholder={
                                                    rule.match_kind === 'keyword' ? 'code, debug, refactor' :
                                                        rule.match_kind === 'context_length' ? '32000' :
                                                            'anthropic'
                                                }
                                                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-border/40 text-sm focus:outline-none focus:ring-1 focus:ring-violet-500/50 placeholder:text-muted-foreground/40"
                                            />
                                        </div>
                                    )}
                                </div>

                                {/* Row 3: Target model + provider */}
                                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                                    <div className="space-y-1.5">
                                        <label className="text-[11px] font-medium uppercase tracking-widest text-muted-foreground/60">
                                            Target Model
                                        </label>
                                        <input
                                            type="text"
                                            value={rule.target_model}
                                            onChange={e => onUpdate(rule.id, { target_model: e.target.value })}
                                            placeholder="gpt-4o, claude-sonnet-4-20250514, etc."
                                            className="w-full px-3 py-2 rounded-lg bg-white/5 border border-border/40 text-sm focus:outline-none focus:ring-1 focus:ring-violet-500/50 placeholder:text-muted-foreground/40"
                                        />
                                    </div>
                                    <div className="space-y-1.5">
                                        <label className="text-[11px] font-medium uppercase tracking-widest text-muted-foreground/60">
                                            Provider Override <span className="text-muted-foreground/30">(optional)</span>
                                        </label>
                                        <input
                                            type="text"
                                            value={rule.target_provider ?? ''}
                                            onChange={e => onUpdate(rule.id, { target_provider: e.target.value || null })}
                                            placeholder="openai, anthropic, openrouter..."
                                            className="w-full px-3 py-2 rounded-lg bg-white/5 border border-border/40 text-sm focus:outline-none focus:ring-1 focus:ring-violet-500/50 placeholder:text-muted-foreground/40"
                                        />
                                    </div>
                                </div>

                                {/* Delete button */}
                                <div className="flex justify-end pt-1">
                                    <button
                                        onClick={() => onDelete(rule.id)}
                                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-red-400 hover:text-red-300 hover:bg-red-500/10 transition-colors"
                                    >
                                        <Trash2 className="w-3.5 h-3.5" /> Delete Rule
                                    </button>
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </motion.div>
        </Reorder.Item>
    );
}

// ── Main Component ───────────────────────────────────────────────────

export function OpenClawRouting() {
    const [smartRoutingEnabled, setSmartRoutingEnabled] = useState(false);
    const [rules, setRules] = useState<openclaw.RoutingRule[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [toggling, setToggling] = useState(false);
    const [saving, setSaving] = useState(false);
    const [expandedId, setExpandedId] = useState<string | null>(null);
    const [hasUnsaved, setHasUnsaved] = useState(false);
    const originalRef = useRef<string>('');

    // Load rules on mount
    useEffect(() => {
        openclaw.getRoutingRules()
            .then(resp => {
                setSmartRoutingEnabled(resp.smart_routing_enabled);
                setRules(resp.rules);
                originalRef.current = JSON.stringify(resp.rules);
            })
            .catch(() => {
                // Fallback to toggle-only mode
                openclaw.getRoutingConfig()
                    .then(cfg => setSmartRoutingEnabled(cfg.smart_routing_enabled))
                    .catch(() => { });
            })
            .finally(() => setIsLoading(false));
    }, []);

    // Track unsaved changes
    useEffect(() => {
        setHasUnsaved(JSON.stringify(rules) !== originalRef.current);
    }, [rules]);

    const handleToggle = async () => {
        const next = !smartRoutingEnabled;
        setToggling(true);
        setSmartRoutingEnabled(next);
        try {
            await openclaw.setRoutingConfig(next);
            toast.success(next ? '🧠 Smart Routing enabled' : 'Smart Routing disabled');
        } catch (e) {
            setSmartRoutingEnabled(!next);
            toast.error(`Failed to toggle: ${e}`);
        } finally {
            setToggling(false);
        }
    };

    const handleSave = async () => {
        setSaving(true);
        try {
            // Re-index priorities before saving
            const reindexed = rules.map((r, i) => ({ ...r, priority: i }));
            await openclaw.saveRoutingRules(reindexed);
            setRules(reindexed);
            originalRef.current = JSON.stringify(reindexed);
            setHasUnsaved(false);
            toast.success(`Saved ${reindexed.length} routing rule${reindexed.length !== 1 ? 's' : ''}`);
        } catch (e) {
            toast.error(`Failed to save rules: ${e}`);
        } finally {
            setSaving(false);
        }
    };

    const handleAddRule = () => {
        const newRule = createEmptyRule(rules.length);
        setRules(prev => [...prev, newRule]);
        setExpandedId(newRule.id);
    };

    const handleUpdateRule = useCallback((id: string, patch: Partial<openclaw.RoutingRule>) => {
        setRules(prev => prev.map(r => r.id === id ? { ...r, ...patch } : r));
    }, []);

    const handleDeleteRule = useCallback((id: string) => {
        setRules(prev => prev.filter(r => r.id !== id));
        if (expandedId === id) setExpandedId(null);
    }, [expandedId]);

    const handleToggleExpand = useCallback((id: string) => {
        setExpandedId(prev => prev === id ? null : id);
    }, []);

    const handleReorder = (newOrder: openclaw.RoutingRule[]) => {
        setRules(newOrder);
    };

    const enabledCount = rules.filter(r => r.enabled).length;

    return (
        <motion.div
            className="flex-1 overflow-y-auto p-8 space-y-8"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-violet-500/10 border border-violet-500/20">
                        <GitBranch className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">LLM Routing</h1>
                        <p className="text-xs text-muted-foreground">
                            Configure how requests are routed across models and providers
                        </p>
                    </div>
                </div>

                {/* Save button */}
                <AnimatePresence>
                    {hasUnsaved && (
                        <motion.button
                            initial={{ opacity: 0, scale: 0.9 }}
                            animate={{ opacity: 1, scale: 1 }}
                            exit={{ opacity: 0, scale: 0.9 }}
                            onClick={handleSave}
                            disabled={saving}
                            className={cn(
                                "flex items-center gap-2 px-4 py-2 rounded-xl text-sm font-semibold transition-all",
                                "bg-violet-500 hover:bg-violet-400 text-white shadow-lg shadow-violet-500/20",
                                saving && "opacity-60 cursor-wait"
                            )}
                        >
                            {saving ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
                            Save Changes
                        </motion.button>
                    )}
                </AnimatePresence>
            </div>

            {/* Smart Routing Toggle Card */}
            <motion.div
                initial={{ opacity: 0, y: 5 }}
                animate={{ opacity: 1, y: 0 }}
                className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md overflow-hidden"
            >
                <div className="p-6 space-y-5">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-4">
                            <div className={cn(
                                "p-3 rounded-xl border transition-all duration-300",
                                smartRoutingEnabled
                                    ? "bg-violet-500/10 border-violet-500/20 text-primary"
                                    : "bg-white/5 border-border/40 text-muted-foreground"
                            )}>
                                <Zap className="w-5 h-5" />
                            </div>
                            <div>
                                <h2 className="font-semibold text-base">Smart Routing</h2>
                                <p className="text-xs text-muted-foreground mt-0.5">
                                    Automatically route requests to the optimal model based on task complexity
                                </p>
                            </div>
                        </div>

                        {/* Toggle Switch */}
                        <button
                            onClick={handleToggle}
                            disabled={isLoading || toggling}
                            className={cn(
                                "relative w-12 h-6 rounded-full transition-all duration-300 shrink-0",
                                smartRoutingEnabled
                                    ? "bg-violet-500"
                                    : "bg-zinc-700",
                                (isLoading || toggling) && "opacity-50 cursor-wait"
                            )}
                        >
                            <motion.div
                                className="absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white shadow-md"
                                animate={{ x: smartRoutingEnabled ? 24 : 0 }}
                                transition={{ type: "spring", stiffness: 500, damping: 30 }}
                            />
                        </button>
                    </div>

                    {/* Status indicator */}
                    <div className={cn(
                        "flex items-center gap-2 px-3 py-2 rounded-lg text-xs font-medium transition-all",
                        smartRoutingEnabled
                            ? "bg-violet-500/10 text-primary border border-violet-500/20"
                            : "bg-zinc-500/10 text-muted-foreground border border-zinc-500/20"
                    )}>
                        {isLoading ? (
                            <><RefreshCw className="w-3.5 h-3.5 animate-spin" /> Loading configuration…</>
                        ) : smartRoutingEnabled ? (
                            <><CheckCircle2 className="w-3.5 h-3.5" /> Smart routing is active — {enabledCount} rule{enabledCount !== 1 ? 's' : ''} enabled</>
                        ) : (
                            <><Info className="w-3.5 h-3.5" /> Smart routing is disabled — all requests go to the default model</>
                        )}
                    </div>
                </div>
            </motion.div>

            {/* How It Works */}
            <motion.div
                initial={{ opacity: 0, y: 5 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.1 }}
                className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-6 space-y-4"
            >
                <h3 className="text-sm font-bold uppercase tracking-widest text-muted-foreground/60 flex items-center gap-2">
                    <Cpu className="w-3.5 h-3.5" />
                    How Smart Routing Works
                </h3>

                <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    {[
                        {
                            title: 'Analyze',
                            description: 'Each request is analyzed for complexity, context length, and required capabilities.',
                            icon: Layers,
                            color: 'text-blue-400 bg-blue-500/10 border-blue-500/20',
                        },
                        {
                            title: 'Route',
                            description: 'The routing engine selects the optimal model based on cost, speed, and quality tradeoffs.',
                            icon: GitBranch,
                            color: 'text-primary bg-violet-500/10 border-violet-500/20',
                        },
                        {
                            title: 'Execute',
                            description: 'The request is sent to the selected model with automatic fallback if the primary provider fails.',
                            icon: Zap,
                            color: 'text-primary bg-emerald-500/10 border-emerald-500/20',
                        },
                    ].map((step, i) => (
                        <div key={step.title} className="relative">
                            <div className="p-4 rounded-xl border border-white/5 bg-white/[0.02] space-y-3">
                                <div className={cn("p-2 rounded-lg border w-fit", step.color)}>
                                    <step.icon className="w-4 h-4" />
                                </div>
                                <div>
                                    <h4 className="text-sm font-semibold">{step.title}</h4>
                                    <p className="text-xs text-muted-foreground mt-1 leading-relaxed">{step.description}</p>
                                </div>
                            </div>
                            {i < 2 && (
                                <ArrowRight className="hidden md:block absolute -right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground/30 z-10" />
                            )}
                        </div>
                    ))}
                </div>
            </motion.div>

            {/* ── Routing Rules Builder ─────────────────────────────────────── */}
            <motion.div
                initial={{ opacity: 0, y: 5 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.15 }}
                className="space-y-4"
            >
                <div className="flex items-center justify-between">
                    <h3 className="text-sm font-bold uppercase tracking-widest text-muted-foreground/60 flex items-center gap-2">
                        <Layers className="w-3.5 h-3.5" />
                        Routing Rules
                        {rules.length > 0 && (
                            <span className="ml-1 px-1.5 py-0.5 rounded-md bg-white/5 text-[10px] font-medium text-muted-foreground">
                                {rules.length}
                            </span>
                        )}
                    </h3>
                    <button
                        onClick={handleAddRule}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-violet-500/10 text-primary hover:bg-violet-500/20 border border-violet-500/20 transition-colors"
                    >
                        <Plus className="w-3.5 h-3.5" /> Add Rule
                    </button>
                </div>

                {!smartRoutingEnabled && rules.length > 0 && (
                    <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-amber-500/5 text-muted-foreground border border-amber-500/10 text-xs">
                        <AlertCircle className="w-3.5 h-3.5 shrink-0" />
                        Smart routing is currently disabled — rules will not be evaluated until you enable it.
                    </div>
                )}

                {rules.length === 0 ? (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        className="rounded-2xl border border-dashed border-border/40 bg-white/[0.01] p-8 flex flex-col items-center justify-center text-center space-y-3"
                    >
                        <div className="p-3 rounded-xl bg-white/5 border border-border/40">
                            <GitBranch className="w-6 h-6 text-muted-foreground/40" />
                        </div>
                        <div>
                            <p className="text-sm font-medium text-muted-foreground">No routing rules defined</p>
                            <p className="text-xs text-muted-foreground/60 mt-1 max-w-sm">
                                Add rules to control which model handles each type of request.
                                Rules are evaluated top-to-bottom — the first matching rule wins.
                            </p>
                        </div>
                        <button
                            onClick={handleAddRule}
                            className="flex items-center gap-1.5 px-4 py-2 rounded-xl text-sm font-medium bg-violet-500 text-white hover:bg-violet-400 transition-colors mt-2"
                        >
                            <Plus className="w-4 h-4" /> Create First Rule
                        </button>
                    </motion.div>
                ) : (
                    <Reorder.Group
                        axis="y"
                        values={rules}
                        onReorder={handleReorder}
                        className="space-y-2"
                    >
                        <AnimatePresence>
                            {rules.map((rule, i) => (
                                <RuleCard
                                    key={rule.id}
                                    rule={rule}
                                    index={i}
                                    onUpdate={handleUpdateRule}
                                    onDelete={handleDeleteRule}
                                    isExpanded={expandedId === rule.id}
                                    onToggleExpand={handleToggleExpand}
                                />
                            ))}
                        </AnimatePresence>
                    </Reorder.Group>
                )}

                {rules.length > 0 && (
                    <p className="text-[10px] text-muted-foreground/40 text-center pt-2">
                        Rules are evaluated in order — drag to reorder. First matching rule wins.
                    </p>
                )}
            </motion.div>
        </motion.div>
    );
}
