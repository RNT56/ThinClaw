import { useCallback, useEffect, useState, type ReactNode } from 'react';
import {
    AlertTriangle,
    Ban,
    CheckCircle2,
    Eye,
    Network,
    RefreshCw,
    ShieldCheck,
    type LucideIcon,
} from 'lucide-react';

import { commands, type SecurityPosture } from '../../lib/bindings';
import { cn } from '../../lib/utils';

const REFRESH_INTERVAL_MS = 5_000;

export function SecurityPosturePanel() {
    const [posture, setPosture] = useState<SecurityPosture | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    const refresh = useCallback(async (showSpinner = false) => {
        if (showSpinner) setLoading(true);
        try {
            const result = await commands.thinclawSecurityPosture();
            if (result.status === 'error') {
                setError(String(result.error));
                return;
            }
            setPosture(result.data);
            setError(null);
        } catch (cause) {
            setError(cause instanceof Error ? cause.message : String(cause));
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        void refresh();
        const interval = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
        return () => window.clearInterval(interval);
    }, [refresh]);

    if (loading && !posture) {
        return <SecurityPanelSkeleton />;
    }

    if (!posture) {
        return (
            <Notice tone="danger" title="Security evidence could not be loaded">
                {error ?? 'The Desktop backend returned no posture data.'}
            </Notice>
        );
    }

    return (
        <div className="space-y-6">
            <div className="flex items-start justify-between gap-4">
                <div>
                    <div className="flex items-center gap-2" aria-live="polite">
                        <span className={cn(
                            'h-2.5 w-2.5 rounded-full',
                            posture.evidence_available ? 'bg-emerald-500' : 'bg-amber-500',
                        )} />
                        <span className="text-sm font-semibold">
                            {posture.evidence_available ? 'Live local evidence' : 'Evidence unavailable'}
                        </span>
                        <span className="rounded-full bg-muted px-2 py-0.5 text-[11px] uppercase tracking-wide text-muted-foreground">
                            {posture.runtime_mode}
                        </span>
                    </div>
                    <p className="mt-2 max-w-2xl text-sm text-muted-foreground">
                        This view is read-only. It reports effective runtime controls and stores only rule metadata—never prompts, tool parameters, outputs, or secrets.
                    </p>
                </div>
                <button
                    type="button"
                    onClick={() => void refresh(true)}
                    disabled={loading}
                    className="inline-flex items-center gap-2 rounded-lg border border-border bg-background px-3 py-2 text-sm font-medium transition-colors hover:bg-muted disabled:opacity-50"
                >
                    <RefreshCw className={cn('h-4 w-4', loading && 'animate-spin')} />
                    Refresh
                </button>
            </div>

            {error && <Notice tone="danger" title="Last refresh failed">{error}</Notice>}
            {!posture.evidence_available && (
                <Notice tone="warning" title="No authoritative runtime evidence">
                    {posture.unavailable_reason ?? 'The current runtime cannot provide security posture data.'}
                </Notice>
            )}
            {posture.tools.auto_approve_enabled && (
                <Notice tone="warning" title="Automatic tool approval is enabled">
                    Conditional approval prompts may be bypassed. Tools classified as always-approval still require an explicit decision.
                </Notice>
            )}

            <section className="rounded-xl border border-border bg-card p-5">
                <SectionHeading icon={ShieldCheck} title="Sanitizer and policy activity" subtitle="Metadata-only events from the active safety layer" />
                <div className="mt-4 grid grid-cols-2 gap-3 md:grid-cols-4">
                    <Metric label="Sanitized" value={posture.telemetry.sanitized} />
                    <Metric label="Secrets redacted" value={posture.telemetry.redacted} />
                    <Metric label="Blocked" value={posture.telemetry.blocked} />
                    <Metric label="Warnings" value={posture.telemetry.warned} />
                </div>
                <div className="mt-5 border-t border-border/60 pt-4">
                    <h3 className="text-sm font-semibold">Recent decisions</h3>
                    {posture.telemetry.recent_events.length === 0 ? (
                        <p className="mt-2 text-sm text-muted-foreground">No safety-layer decisions have been recorded in this runtime session.</p>
                    ) : (
                        <div className="mt-3 max-h-72 space-y-2 overflow-y-auto pr-1">
                            {posture.telemetry.recent_events.map((event, index) => (
                                <div key={`${event.occurred_at_ms}-${event.reason}-${index}`} className="flex items-start gap-3 rounded-lg bg-muted/45 p-3 text-sm">
                                    <EventIcon action={event.action} />
                                    <div className="min-w-0 flex-1">
                                        <div className="flex flex-wrap items-center gap-2">
                                            <span className="font-medium capitalize">{event.action}</span>
                                            <span className="rounded bg-background/70 px-1.5 py-0.5 text-[10px] uppercase text-muted-foreground">{event.severity}</span>
                                            <span className="text-xs text-muted-foreground">{formatTime(event.occurred_at_ms)}</span>
                                        </div>
                                        <p className="mt-1 break-words text-muted-foreground">{event.reason} · {event.source}</p>
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            </section>

            <section className="rounded-xl border border-border bg-card p-5">
                <SectionHeading icon={Network} title="Sandbox boundary" subtitle="Effective filesystem policy, resource cap, and outbound network allowlist" />
                {posture.sandbox ? (
                    <>
                        <div className="mt-4 grid gap-3 sm:grid-cols-3">
                            <Metric label="State" value={posture.sandbox.enabled ? 'Enabled' : 'Disabled'} />
                            <Metric label="Policy" value={posture.sandbox.policy.replace(/_/g, ' ')} />
                            <Metric label="Limits" value={`${posture.sandbox.memory_limit_mb} MB · ${posture.sandbox.timeout_secs}s`} />
                        </div>
                        <div className="mt-5 border-t border-border/60 pt-4">
                            <h3 className="text-sm font-semibold">Allowed network destinations</h3>
                            {posture.sandbox.network_allowlist.length === 0 ? (
                                <p className="mt-2 text-sm text-muted-foreground">No destinations are allowlisted.</p>
                            ) : (
                                <div className="mt-3 flex flex-wrap gap-2">
                                    {posture.sandbox.network_allowlist.map((host) => (
                                        <code key={host} className="rounded-md border border-border bg-muted/50 px-2 py-1 text-xs">{host}</code>
                                    ))}
                                </div>
                            )}
                        </div>
                    </>
                ) : (
                    <p className="mt-4 text-sm text-muted-foreground">Start the local runtime to inspect the effective sandbox configuration.</p>
                )}
            </section>

            <section className="rounded-xl border border-border bg-card p-5">
                <SectionHeading icon={Eye} title="Tool execution controls" subtitle="Live registry metadata; the retired orphan tracker is not treated as enforcement" />
                <div className="mt-4 grid grid-cols-2 gap-3 lg:grid-cols-5">
                    <Metric label="Registered" value={posture.tools.registered} />
                    <Metric label="Write-capable" value={posture.tools.write_capable} />
                    <Metric label="Always approval" value={posture.tools.always_approval} />
                    <Metric label="Conditional" value={posture.tools.conditional_approval} />
                    <Metric label="Write / coarse never" value={posture.tools.write_without_coarse_approval} />
                </div>
                <div className="mt-5 max-h-96 space-y-2 overflow-y-auto border-t border-border/60 pt-4 pr-1">
                    {posture.tools.reviewed_tools.map((tool) => (
                        <div key={tool.name} className="rounded-lg border border-border/70 p-3">
                            <div className="flex flex-wrap items-center gap-2">
                                <code className="font-semibold text-foreground">{tool.name}</code>
                                <Badge>{tool.side_effect}</Badge>
                                <Badge>approval: {tool.approval_class}</Badge>
                                <Badge>empty call: {tool.empty_params_requirement.replace(/_/g, ' ')}</Badge>
                                {tool.sanitizes_output && <Badge>output sanitized</Badge>}
                            </div>
                            <p className="mt-2 text-sm text-muted-foreground">{tool.reason}</p>
                        </div>
                    ))}
                    {posture.evidence_available && posture.tools.reviewed_tools.length === 0 && (
                        <p className="text-sm text-muted-foreground">No write-capable or approval-gated tools are registered.</p>
                    )}
                </div>
            </section>
        </div>
    );
}

function SecurityPanelSkeleton() {
    return (
        <div className="space-y-5 animate-pulse" aria-label="Loading security posture">
            {[0, 1, 2].map((item) => <div key={item} className="h-40 rounded-xl border border-border bg-muted/30" />)}
        </div>
    );
}

function SectionHeading({ icon: Icon, title, subtitle }: { icon: LucideIcon; title: string; subtitle: string }) {
    return (
        <div className="flex items-start gap-3">
            <div className="rounded-lg bg-primary/10 p-2"><Icon className="h-5 w-5 text-primary" /></div>
            <div><h2 className="font-semibold">{title}</h2><p className="text-sm text-muted-foreground">{subtitle}</p></div>
        </div>
    );
}

function Metric({ label, value }: { label: string; value: string | number }) {
    return <div className="rounded-lg bg-muted/45 p-3"><div className="text-xs text-muted-foreground">{label}</div><div className="mt-1 text-lg font-semibold capitalize">{value}</div></div>;
}

function Badge({ children }: { children: ReactNode }) {
    return <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">{children}</span>;
}

function Notice({ tone, title, children }: { tone: 'warning' | 'danger'; title: string; children: ReactNode }) {
    const Icon = tone === 'danger' ? Ban : AlertTriangle;
    return (
        <div role={tone === 'danger' ? 'alert' : 'status'} className={cn('flex gap-3 rounded-xl border p-4', tone === 'danger' ? 'border-destructive/40 bg-destructive/10' : 'border-amber-500/40 bg-amber-500/10')}>
            <Icon className={cn('mt-0.5 h-5 w-5 shrink-0', tone === 'danger' ? 'text-destructive' : 'text-amber-500')} />
            <div><h2 className="text-sm font-semibold">{title}</h2><p className="mt-1 text-sm text-muted-foreground">{children}</p></div>
        </div>
    );
}

function EventIcon({ action }: { action: string }) {
    if (action === 'blocked') return <Ban className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />;
    if (action === 'warned') return <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />;
    return <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-emerald-500" />;
}

function formatTime(timestamp: number) {
    return new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}
