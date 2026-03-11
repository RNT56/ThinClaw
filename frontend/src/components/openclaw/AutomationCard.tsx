/**
 * AutomationCard — rich, themed card for displaying automation/heartbeat output in chat.
 *
 * Two variants:
 * - `heartbeat`: Rose/pink border, pulse icon, compact layout
 * - `automation`: Blue/indigo border, expandable with full detail
 *
 * Status differentiation:
 * - OK: Emerald badge ✅
 * - Attention: Amber badge 🔔
 * - Failed: Red badge ❌
 * - Running: Blue badge ⏳
 */

import { useState, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Heart,
    Zap,
    ChevronDown,
    AlertTriangle,
    CheckCircle2,
    XCircle,
    Clock,
    Bell,
    Activity,
    Copy,
    Check,
    FileDown,
    FolderOpen,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import { writeAgentWorkspaceFile, revealFile } from '../../lib/openclaw';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

// ── Types ────────────────────────────────────────────────────────────────

export type AutomationVariant = 'heartbeat' | 'automation';
export type AutomationStatus = 'ok' | 'attention' | 'failed' | 'running' | 'dispatched';

export interface AutomationCardProps {
    routineName: string;
    variant: AutomationVariant;
    status: AutomationStatus;
    content: string;
    timestamp?: number;
    toolCount?: number;
    duration?: number;
    className?: string;
}

// ── Status Config ────────────────────────────────────────────────────────

const STATUS_CONFIG: Record<AutomationStatus, {
    icon: typeof CheckCircle2;
    label: string;
    badgeClass: string;
    borderClass: string;
    glowClass: string;
}> = {
    ok: {
        icon: CheckCircle2,
        label: 'All Clear',
        badgeClass: 'bg-emerald-500/15 text-emerald-400 border-emerald-500/30',
        borderClass: 'border-emerald-500/20',
        glowClass: 'shadow-[0_0_15px_rgba(16,185,129,0.08)]',
    },
    attention: {
        icon: Bell,
        label: 'Needs Attention',
        badgeClass: 'bg-amber-500/15 text-amber-400 border-amber-500/30',
        borderClass: 'border-amber-500/25',
        glowClass: 'shadow-[0_0_15px_rgba(245,158,11,0.1)]',
    },
    failed: {
        icon: XCircle,
        label: 'Failed',
        badgeClass: 'bg-red-500/15 text-red-400 border-red-500/30',
        borderClass: 'border-red-500/25',
        glowClass: 'shadow-[0_0_15px_rgba(239,68,68,0.1)]',
    },
    running: {
        icon: Clock,
        label: 'Running',
        badgeClass: 'bg-blue-500/15 text-blue-400 border-blue-500/30',
        borderClass: 'border-blue-500/20',
        glowClass: 'shadow-[0_0_15px_rgba(59,130,246,0.08)]',
    },
    dispatched: {
        icon: Clock,
        label: 'Dispatched',
        badgeClass: 'bg-blue-500/15 text-blue-400 border-blue-500/30',
        borderClass: 'border-blue-500/20',
        glowClass: 'shadow-[0_0_12px_rgba(59,130,246,0.06)]',
    },
};

// ── Variant Config ───────────────────────────────────────────────────────

const VARIANT_CONFIG: Record<AutomationVariant, {
    icon: typeof Heart;
    accentClass: string;
    headerGradient: string;
}> = {
    heartbeat: {
        icon: Heart,
        accentClass: 'text-rose-400',
        headerGradient: 'from-rose-500/10 to-pink-500/5',
    },
    automation: {
        icon: Zap,
        accentClass: 'text-indigo-400',
        headerGradient: 'from-indigo-500/10 to-blue-500/5',
    },
};

// ── Helper ───────────────────────────────────────────────────────────────

function formatDisplayName(routineName: string): string {
    if (routineName === '__heartbeat__') return 'Heartbeat Check';
    return routineName
        .replace(/[-_]/g, ' ')
        .replace(/\b\w/g, (c) => c.toUpperCase());
}

function formatTimestamp(ts?: number): string {
    if (!ts) return '';
    const d = new Date(ts);
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatDuration(ms?: number): string {
    if (!ms) return '';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
}

/**
 * Parse the heartbeat content into individual items for structured display.
 * Handles both markdown bullet lists and prose text.
 */
function parseFindings(content: string): string[] {
    // Split on markdown bullets or numbered items
    const lines = content.split(/\n/).filter(l => l.trim());
    const findings: string[] = [];

    for (const line of lines) {
        const trimmed = line.trim();
        // Match "- item" / "* item" / "1. item"
        const match = trimmed.match(/^[-*•]\s+(.+)$/) || trimmed.match(/^\d+\.\s+(.+)$/);
        if (match) {
            findings.push(match[1]);
        }
    }

    // If no bullet items found, return as a single block
    if (findings.length === 0 && content.trim()) {
        return [content.trim()];
    }

    return findings;
}

// ── Component ────────────────────────────────────────────────────────────

export default function AutomationCard({
    routineName,
    variant,
    status,
    content,
    timestamp,
    toolCount,
    duration,
    className,
}: AutomationCardProps) {
    const [expanded, setExpanded] = useState(status === 'attention' || status === 'failed');
    const [copied, setCopied] = useState(false);
    const [saving, setSaving] = useState(false);
    const [savedPath, setSavedPath] = useState<string | null>(null);

    const statusCfg = STATUS_CONFIG[status];
    const variantCfg = VARIANT_CONFIG[variant];
    const StatusIcon = statusCfg.icon;
    const VariantIcon = variantCfg.icon;
    const displayName = formatDisplayName(routineName);
    const findings = parseFindings(content);
    const hasFindings = findings.length > 0 && content.trim().length > 0;

    // ── Copy action ──────────────────────────────────────────────
    const handleCopy = useCallback(async () => {
        try {
            await navigator.clipboard.writeText(content);
            setCopied(true);
            toast.success('Copied to clipboard');
            setTimeout(() => setCopied(false), 2000);
        } catch {
            toast.error('Failed to copy');
        }
    }, [content]);

    // ── Write to File action ─────────────────────────────────────
    const handleWriteToFile = useCallback(async () => {
        setSaving(true);
        try {
            const sanitized = routineName.replace(/[^a-zA-Z0-9_-]/g, '_').toLowerCase();
            const dateStr = new Date().toISOString().slice(0, 10);
            const timeStr = new Date().toISOString().slice(11, 19).replace(/:/g, '');
            const filename = `reports/${sanitized}_${dateStr}_${timeStr}.md`;

            const header = `# ${displayName} — ${statusCfg.label}\n\n` +
                `> Generated: ${new Date().toLocaleString()}\n\n---\n\n`;
            const absPath = await writeAgentWorkspaceFile(filename, header + content);
            setSavedPath(absPath);
            toast.success(`Saved to ${filename}`, {
                action: {
                    label: 'Reveal',
                    onClick: () => revealFile(absPath),
                },
            });
        } catch (e: any) {
            toast.error(e?.message || 'Failed to save file');
        } finally {
            setSaving(false);
        }
    }, [routineName, displayName, statusCfg.label, content]);

    return (
        <motion.div
            initial={{ opacity: 0, y: 8, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            transition={{ duration: 0.3, ease: 'easeOut' }}
            className={cn(
                'rounded-xl border overflow-hidden',
                'bg-[var(--bg-secondary)]/60 backdrop-blur-sm',
                statusCfg.borderClass,
                statusCfg.glowClass,
                className
            )}
        >
            {/* Header */}
            <button
                onClick={() => setExpanded(!expanded)}
                className={cn(
                    'w-full flex items-center justify-between gap-3 px-4 py-3',
                    'bg-gradient-to-r', variantCfg.headerGradient,
                    'hover:brightness-110 transition-all duration-200',
                    'cursor-pointer'
                )}
            >
                {/* Left: Icon + Name */}
                <div className="flex items-center gap-2.5 min-w-0">
                    <div className={cn(
                        'flex items-center justify-center w-7 h-7 rounded-lg',
                        'bg-[var(--bg-primary)]/40',
                        variant === 'heartbeat' && status !== 'failed' && 'animate-pulse'
                    )}>
                        <VariantIcon className={cn('w-3.5 h-3.5', variantCfg.accentClass)} />
                    </div>
                    <span className="text-[13px] font-semibold text-[var(--text-primary)] truncate">
                        {displayName}
                    </span>
                </div>

                {/* Right: Status badge + Time + Expand */}
                <div className="flex items-center gap-2 shrink-0">
                    {/* Status badge */}
                    <span className={cn(
                        'inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-semibold uppercase tracking-wider border',
                        statusCfg.badgeClass
                    )}>
                        <StatusIcon className="w-2.5 h-2.5" />
                        {statusCfg.label}
                    </span>

                    {/* Timestamp */}
                    {timestamp && (
                        <span className="text-[10px] text-[var(--text-tertiary)] tabular-nums">
                            {formatTimestamp(timestamp)}
                        </span>
                    )}

                    {/* Expand chevron */}
                    {hasFindings && (
                        <motion.div animate={{ rotate: expanded ? 180 : 0 }} transition={{ duration: 0.2 }}>
                            <ChevronDown className="w-3.5 h-3.5 text-[var(--text-tertiary)]" />
                        </motion.div>
                    )}
                </div>
            </button>

            {/* Expandable content */}
            <AnimatePresence initial={false}>
                {expanded && hasFindings && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        transition={{ duration: 0.25, ease: 'easeInOut' }}
                        className="overflow-hidden"
                    >
                        <div className="px-4 py-3 border-t border-[var(--border-primary)]/50">
                            {/* If there are structured findings, render as a list */}
                            {findings.length > 1 ? (
                                <div className="space-y-1.5">
                                    {findings.map((finding, i) => (
                                        <div key={i} className="flex items-start gap-2">
                                            <AlertTriangle className={cn(
                                                'w-3 h-3 mt-0.5 shrink-0',
                                                status === 'attention' ? 'text-amber-400' :
                                                    status === 'failed' ? 'text-red-400' :
                                                        'text-emerald-400'
                                            )} />
                                            <span className="text-[12px] text-[var(--text-secondary)] leading-relaxed">
                                                {finding}
                                            </span>
                                        </div>
                                    ))}
                                </div>
                            ) : (
                                /* Single block of content — render as markdown */
                                <div className="text-[12px] text-[var(--text-secondary)] prose prose-invert prose-sm max-w-none
                                    [&_p]:my-1 [&_ul]:my-1 [&_li]:my-0.5 [&_code]:text-[11px] [&_code]:bg-[var(--bg-primary)]/40 [&_code]:px-1 [&_code]:rounded">
                                    <ReactMarkdown remarkPlugins={[remarkGfm]}>
                                        {content}
                                    </ReactMarkdown>
                                </div>
                            )}
                        </div>

                        {/* Footer: stats + action buttons */}
                        <div className="px-4 py-2 border-t border-[var(--border-primary)]/30 flex items-center justify-between">
                            <div className="flex items-center gap-3">
                                {toolCount && (
                                    <span className="inline-flex items-center gap-1 text-[10px] text-[var(--text-tertiary)]">
                                        <Activity className="w-2.5 h-2.5" />
                                        {toolCount} tools
                                    </span>
                                )}
                                {duration && (
                                    <span className="inline-flex items-center gap-1 text-[10px] text-[var(--text-tertiary)]">
                                        <Clock className="w-2.5 h-2.5" />
                                        {formatDuration(duration)}
                                    </span>
                                )}
                            </div>

                            {/* Action buttons */}
                            <div className="flex items-center gap-1">
                                <button
                                    onClick={(e) => { e.stopPropagation(); handleCopy(); }}
                                    title="Copy to clipboard"
                                    className={cn(
                                        'p-1.5 rounded-lg transition-all duration-200',
                                        'hover:bg-[var(--bg-primary)]/60 text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]',
                                        copied && 'text-emerald-400 hover:text-emerald-400'
                                    )}
                                >
                                    {copied
                                        ? <Check className="w-3 h-3" />
                                        : <Copy className="w-3 h-3" />
                                    }
                                </button>

                                {savedPath ? (
                                    <button
                                        onClick={(e) => { e.stopPropagation(); revealFile(savedPath); }}
                                        title="Reveal saved file"
                                        className="p-1.5 rounded-lg transition-all duration-200 hover:bg-[var(--bg-primary)]/60 text-emerald-400 hover:text-emerald-300"
                                    >
                                        <FolderOpen className="w-3 h-3" />
                                    </button>
                                ) : (
                                    <button
                                        onClick={(e) => { e.stopPropagation(); handleWriteToFile(); }}
                                        disabled={saving}
                                        title="Save to agent_workspace"
                                        className={cn(
                                            'p-1.5 rounded-lg transition-all duration-200',
                                            'hover:bg-[var(--bg-primary)]/60 text-[var(--text-tertiary)] hover:text-[var(--text-secondary)]',
                                            saving && 'opacity-50'
                                        )}
                                    >
                                        <FileDown className="w-3 h-3" />
                                    </button>
                                )}
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}
