import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import {
    Activity, RefreshCw, CheckCircle2, XCircle, AlertTriangle, MinusCircle,
    Stethoscope, Server, Database, HardDrive, Wrench, Anchor, Puzzle, Sparkles
} from 'lucide-react';
import * as thinclawApi from '../../lib/thinclaw';
import { toast } from 'sonner';

const STATUS_CONFIG: Record<string, { icon: typeof CheckCircle2; color: string; bg: string }> = {
    pass: { icon: CheckCircle2, color: 'text-primary', bg: 'bg-emerald-500/10 border-emerald-500/30' },
    fail: { icon: XCircle, color: 'text-red-400', bg: 'bg-red-500/10 border-red-500/30' },
    warn: { icon: AlertTriangle, color: 'text-muted-foreground', bg: 'bg-amber-500/10 border-amber-500/30' },
    skip: { icon: MinusCircle, color: 'text-muted-foreground/60', bg: 'bg-muted/30 border-border/40' },
};

const CHECK_ICONS: Record<string, typeof Server> = {
    'ThinClaw Engine': Server,
    'Database': Database,
    'Workspace': HardDrive,
    'Tool Registry': Wrench,
    'Hook Registry': Anchor,
    'Extensions': Puzzle,
    'Skills': Sparkles,
};

export function ThinClawDoctor() {
    const [diagnostics, setDiagnostics] = useState<thinclawApi.DiagnosticsResponse | null>(null);
    const [loading, setLoading] = useState(false);
    const [lastRun, setLastRun] = useState<Date | null>(null);

    const runDiagnostics = useCallback(async () => {
        setLoading(true);
        try {
            const resp = await thinclawApi.runDiagnostics();
            setDiagnostics(resp);
            setLastRun(new Date());
        } catch (e) {
            toast.error('Diagnostics failed', { description: String(e) });
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => { runDiagnostics(); }, [runDiagnostics]);

    const total = diagnostics ? diagnostics.passed + diagnostics.failed + diagnostics.skipped : 0;
    const healthPercent = total > 0 ? Math.round((diagnostics!.passed / total) * 100) : 0;

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Header */}
            <div className="flex-shrink-0 px-5 pt-5 pb-3">
                <div className="flex items-center justify-between mb-4">
                    <div className="flex items-center gap-3">
                        <div className="w-9 h-9 rounded-xl bg-gradient-to-br from-rose-500/20 to-pink-500/20 border border-rose-500/30 flex items-center justify-center">
                            <Stethoscope className="w-4.5 h-4.5 text-rose-400" />
                        </div>
                        <div>
                            <h2 className="text-base font-semibold text-foreground">System Doctor</h2>
                            <p className="text-xs text-muted-foreground/60">
                                {lastRun ? `Last run: ${lastRun.toLocaleTimeString()}` : 'Not yet run'}
                            </p>
                        </div>
                    </div>
                    <button
                        onClick={runDiagnostics}
                        disabled={loading}
                        className="px-3 py-1.5 rounded-lg bg-rose-500/10 border border-rose-500/30 text-rose-600 dark:text-rose-300 text-xs font-medium hover:bg-rose-500/20 disabled:opacity-50 transition-all flex items-center gap-1.5"
                    >
                        <RefreshCw className={`w-3 h-3 ${loading ? 'animate-spin' : ''}`} />
                        {loading ? 'Running...' : 'Re-run'}
                    </button>
                </div>

                {/* Health Bar */}
                {diagnostics && (
                    <motion.div
                        initial={{ opacity: 0, y: -10 }}
                        animate={{ opacity: 1, y: 0 }}
                        className="p-3 rounded-xl bg-muted/10 border border-border/30 mb-2"
                    >
                        <div className="flex items-center justify-between mb-2">
                            <div className="flex items-center gap-4">
                                <span className="text-2xl font-bold text-foreground">{healthPercent}%</span>
                                <span className="text-xs text-muted-foreground/60 uppercase tracking-wider">System Health</span>
                            </div>
                            <div className="flex items-center gap-3 text-xs">
                                <span className="text-primary">✓ {diagnostics.passed}</span>
                                <span className="text-red-400">✗ {diagnostics.failed}</span>
                                <span className="text-muted-foreground/60">— {diagnostics.skipped}</span>
                            </div>
                        </div>
                        <div className="h-2 bg-muted/30 rounded-full overflow-hidden">
                            <motion.div
                                initial={{ width: 0 }}
                                animate={{ width: `${healthPercent}%` }}
                                transition={{ duration: 0.8, ease: 'easeOut' }}
                                className={`h-full rounded-full ${healthPercent >= 80 ? 'bg-gradient-to-r from-emerald-500 to-emerald-400' :
                                    healthPercent >= 50 ? 'bg-gradient-to-r from-amber-500 to-amber-400' :
                                        'bg-gradient-to-r from-red-500 to-red-400'
                                    }`}
                            />
                        </div>
                    </motion.div>
                )}
            </div>

            {/* Checks */}
            <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-2">
                {loading && !diagnostics ? (
                    <div className="flex items-center justify-center py-16 text-muted-foreground/60">
                        <Activity className="w-6 h-6 animate-pulse mr-2" />
                        Running diagnostics...
                    </div>
                ) : diagnostics ? (
                    diagnostics.checks.map((check, i) => {
                        const cfg = STATUS_CONFIG[check.status] || STATUS_CONFIG.skip;
                        const StatusIcon = cfg.icon;
                        const CheckIcon = CHECK_ICONS[check.name] || Activity;
                        return (
                            <motion.div
                                key={check.name}
                                initial={{ opacity: 0, x: -20 }}
                                animate={{ opacity: 1, x: 0 }}
                                transition={{ delay: i * 0.06 }}
                                className={`p-3 rounded-lg border ${cfg.bg} transition-all`}
                            >
                                <div className="flex items-center gap-3">
                                    <CheckIcon className={`w-4 h-4 ${cfg.color} flex-shrink-0`} />
                                    <div className="flex-1 min-w-0">
                                        <div className="flex items-center gap-2">
                                            <span className="text-sm font-medium text-foreground">{check.name}</span>
                                            <StatusIcon className={`w-3.5 h-3.5 ${cfg.color}`} />
                                        </div>
                                        <p className="text-xs text-muted-foreground/60 mt-0.5">{check.detail}</p>
                                    </div>
                                    <span className={`text-[10px] uppercase tracking-wider font-mono ${cfg.color}`}>
                                        {check.status}
                                    </span>
                                </div>
                            </motion.div>
                        );
                    })
                ) : (
                    <div className="flex flex-col items-center justify-center py-16 text-muted-foreground/60">
                        <Stethoscope className="w-8 h-8 mb-3 opacity-30" />
                        <p className="text-sm">Click "Re-run" to start diagnostics</p>
                    </div>
                )}
            </div>
        </div>
    );
}
