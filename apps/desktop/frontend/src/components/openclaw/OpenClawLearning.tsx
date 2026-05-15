import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { motion } from 'framer-motion';
import {
    AlertTriangle, BrainCircuit, Check, ClipboardCheck, FileCode2, History,
    Lightbulb, RefreshCw, RotateCcw, ShieldAlert, ThumbsDown, ThumbsUp, X
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';

type JsonMap = Record<string, any>;

function arrayFrom(data: unknown, key: string): JsonMap[] {
    const value = (data as JsonMap | null)?.[key];
    return Array.isArray(value) ? value as JsonMap[] : [];
}

function unavailableReason(data: unknown): string | null {
    const value = data as JsonMap | null;
    return value?.available === false ? String(value.reason || 'Unavailable') : null;
}

function text(value: unknown, fallback = 'Unknown') {
    if (value === null || value === undefined || value === '') return fallback;
    return String(value);
}

function Stat({ icon: Icon, label, value, tone = 'text-primary' }: { icon: any; label: string; value: string | number; tone?: string }) {
    return (
        <div className="rounded-xl border border-border/40 bg-card/30 p-4">
            <div className="flex items-center gap-2 text-[10px] uppercase font-bold tracking-widest text-muted-foreground">
                <Icon className={cn("w-3.5 h-3.5", tone)} />
                {label}
            </div>
            <div className={cn("mt-2 text-2xl font-bold tabular-nums", tone)}>{value}</div>
        </div>
    );
}

function Panel({ title, icon: Icon, children }: { title: string; icon: any; children: ReactNode }) {
    return (
        <section className="rounded-xl border border-border/40 bg-card/30 p-5 min-h-0">
            <div className="flex items-center gap-2 mb-4">
                <Icon className="w-4 h-4 text-primary" />
                <h2 className="text-sm font-bold">{title}</h2>
            </div>
            {children}
        </section>
    );
}

function Empty({ text }: { text: string }) {
    return <div className="text-xs text-muted-foreground py-6 text-center border border-dashed border-border/40 rounded-lg">{text}</div>;
}

export function OpenClawLearning() {
    const [loading, setLoading] = useState(true);
    const [refreshing, setRefreshing] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [data, setData] = useState<Record<string, unknown>>({});
    const [proposalFilter, setProposalFilter] = useState<string | null>(null);
    const [outcomeFilter, setOutcomeFilter] = useState<string | null>('open');
    const [rollbackForm, setRollbackForm] = useState({ artifactType: '', artifactName: '', reason: '' });

    const load = useCallback(async () => {
        setRefreshing(true);
        setError(null);
        try {
            const [
                status,
                history,
                candidates,
                artifactVersions,
                providers,
                proposals,
                outcomes,
                rollbacks,
            ] = await Promise.all([
                openclaw.getLearningStatus(50),
                openclaw.getLearningHistory(50),
                openclaw.getLearningCandidates(50),
                openclaw.getLearningArtifactVersions(50),
                openclaw.getLearningProviderHealth(),
                openclaw.getLearningCodeProposals(proposalFilter, 50),
                openclaw.getLearningOutcomes(outcomeFilter, 50),
                openclaw.getLearningRollbacks(50),
            ]);
            setData({ status, history, candidates, artifactVersions, providers, proposals, outcomes, rollbacks });
        } catch (err) {
            setError(String(err));
        } finally {
            setLoading(false);
            setRefreshing(false);
        }
    }, [outcomeFilter, proposalFilter]);

    useEffect(() => {
        load();
    }, [load]);

    const status = (data.status || {}) as JsonMap;
    const recent = (status.recent || {}) as JsonMap;
    const history = arrayFrom(data.history, 'events');
    const evaluations = arrayFrom(data.history, 'evaluations');
    const candidates = arrayFrom(data.candidates, 'candidates');
    const versions = arrayFrom(data.artifactVersions, 'versions');
    const providers = arrayFrom(data.providers, 'providers');
    const proposals = arrayFrom(data.proposals, 'proposals');
    const outcomes = arrayFrom(data.outcomes, 'outcomes');
    const rollbacks = arrayFrom(data.rollbacks, 'rollbacks');
    const unavailable = useMemo(() => [
        unavailableReason(data.status),
        unavailableReason(data.history),
        unavailableReason(data.candidates),
        unavailableReason(data.artifactVersions),
        unavailableReason(data.providers),
        unavailableReason(data.proposals),
        unavailableReason(data.outcomes),
        unavailableReason(data.rollbacks),
    ].filter(Boolean) as string[], [data]);

    const runAction = async (label: string, action: () => Promise<unknown>) => {
        const id = toast.loading(label);
        try {
            const result = await action();
            const reason = unavailableReason(result);
            if (reason) toast.warning(reason, { id });
            else toast.success('Updated', { id });
            await load();
        } catch (err) {
            toast.error(String(err), { id });
        }
    };

    const submitRollback = async () => {
        if (!rollbackForm.artifactType.trim() || !rollbackForm.artifactName.trim() || !rollbackForm.reason.trim()) {
            toast.error('Artifact type, artifact name, and reason are required.');
            return;
        }
        await runAction('Recording rollback', () => openclaw.recordLearningRollback(
            rollbackForm.artifactType.trim(),
            rollbackForm.artifactName.trim(),
            rollbackForm.reason.trim(),
        ));
        setRollbackForm({ artifactType: '', artifactName: '', reason: '' });
    };

    if (loading) {
        return (
            <div className="flex-1 flex items-center justify-center">
                <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
            </div>
        );
    }

    return (
        <motion.div className="flex-1 overflow-y-auto p-8 space-y-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-sky-500/10 border border-sky-500/20">
                        <BrainCircuit className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Learning Review</h1>
                        <p className="text-xs text-muted-foreground">Learning status, history, candidates, outcomes, proposals, provider health, and rollbacks</p>
                    </div>
                </div>
                <button
                    onClick={load}
                    className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                    title="Refresh learning"
                >
                    <RefreshCw className={cn("w-3.5 h-3.5", refreshing && "animate-spin")} />
                </button>
            </div>

            {error && (
                <div className="flex items-center gap-3 rounded-xl border border-red-500/20 bg-red-500/10 p-4 text-sm text-red-300">
                    <AlertTriangle className="w-4 h-4 shrink-0" />
                    {error}
                </div>
            )}

            {unavailable.length > 0 && (
                <div className="flex items-start gap-3 rounded-xl border border-amber-500/20 bg-amber-500/10 p-4 text-xs text-amber-200">
                    <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
                    <div className="space-y-1">
                        {Array.from(new Set(unavailable)).map(reason => <div key={reason}>{reason}</div>)}
                    </div>
                </div>
            )}

            <div className="grid grid-cols-2 lg:grid-cols-5 gap-3">
                <Stat icon={BrainCircuit} label="Enabled" value={status.enabled === true ? 'Yes' : 'No'} tone={status.enabled === true ? 'text-primary' : 'text-muted-foreground'} />
                <Stat icon={Lightbulb} label="Candidates" value={recent.candidates ?? candidates.length} />
                <Stat icon={FileCode2} label="Proposals" value={recent.code_proposals ?? proposals.length} />
                <Stat icon={ClipboardCheck} label="Open Outcomes" value={status.outcomes_open ?? outcomes.length} />
                <Stat icon={RotateCcw} label="Rollbacks" value={recent.rollbacks ?? rollbacks.length} tone="text-amber-300" />
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-2 gap-5">
                <Panel title="Code Proposals" icon={FileCode2}>
                    <div className="flex gap-2 mb-3">
                        {([null, 'pending', 'approved', 'rejected'] as const).map(filter => (
                            <button
                                key={filter || 'all'}
                                onClick={() => setProposalFilter(filter)}
                                className={cn("px-2.5 py-1 rounded-md text-[10px] font-bold uppercase border", proposalFilter === filter ? "border-primary/40 bg-primary/10 text-primary" : "border-border/40 text-muted-foreground hover:text-foreground")}
                            >
                                {filter || 'all'}
                            </button>
                        ))}
                    </div>
                    {proposals.length === 0 ? <Empty text="No code proposals." /> : (
                        <div className="space-y-2">
                            {proposals.map((proposal, index) => {
                                const id = String(proposal.id || '');
                                return (
                                    <div key={id || index} className="rounded-lg border border-border/40 p-3">
                                        <div className="flex items-start justify-between gap-3">
                                            <div className="min-w-0">
                                                <div className="text-sm font-semibold truncate">{text(proposal.title, 'Code proposal')}</div>
                                                <div className="text-[11px] text-muted-foreground line-clamp-2">{text(proposal.rationale, 'No rationale')}</div>
                                            </div>
                                            <span className="text-[10px] uppercase font-bold text-muted-foreground">{text(proposal.status)}</span>
                                        </div>
                                        <div className="mt-3 flex flex-wrap gap-2">
                                            <button onClick={() => id && runAction('Approving proposal', () => openclaw.reviewLearningCodeProposal(id, 'approve'))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-300 border border-emerald-500/20">
                                                <Check className="w-3 h-3" />
                                                Approve
                                            </button>
                                            <button onClick={() => id && runAction('Rejecting proposal', () => openclaw.reviewLearningCodeProposal(id, 'reject'))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-red-500/10 hover:bg-red-500/20 text-red-300 border border-red-500/20">
                                                <X className="w-3 h-3" />
                                                Reject
                                            </button>
                                        </div>
                                        {Array.isArray(proposal.target_files) && proposal.target_files.length > 0 && (
                                            <div className="mt-2 text-[10px] text-muted-foreground truncate">{proposal.target_files.join(', ')}</div>
                                        )}
                                    </div>
                                );
                            })}
                        </div>
                    )}
                </Panel>

                <Panel title="Outcomes" icon={ClipboardCheck}>
                    <div className="flex items-center justify-between gap-3 mb-3">
                        <div className="flex gap-2">
                            {([null, 'open', 'evaluated', 'dismissed'] as const).map(filter => (
                                <button
                                    key={filter || 'all'}
                                    onClick={() => setOutcomeFilter(filter)}
                                    className={cn("px-2.5 py-1 rounded-md text-[10px] font-bold uppercase border", outcomeFilter === filter ? "border-primary/40 bg-primary/10 text-primary" : "border-border/40 text-muted-foreground hover:text-foreground")}
                                >
                                    {filter || 'all'}
                                </button>
                            ))}
                        </div>
                        <button onClick={() => runAction('Evaluating outcomes', () => openclaw.evaluateLearningOutcomes())} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5">
                            <RefreshCw className="w-3 h-3" />
                            Evaluate
                        </button>
                    </div>
                    {outcomes.length === 0 ? <Empty text="No outcomes in this filter." /> : (
                        <div className="space-y-2">
                            {outcomes.map((outcome, index) => {
                                const id = String(outcome.id || '');
                                return (
                                    <div key={id || index} className="rounded-lg border border-border/40 p-3">
                                        <div className="flex items-start justify-between gap-3">
                                            <div className="min-w-0">
                                                <div className="text-sm font-semibold truncate">{text(outcome.summary, 'Outcome contract')}</div>
                                                <div className="text-[11px] text-muted-foreground truncate">{text(outcome.contract_type)} · {text(outcome.source_kind)}</div>
                                            </div>
                                            <span className="text-[10px] uppercase font-bold text-muted-foreground">{text(outcome.status)}</span>
                                        </div>
                                        <div className="mt-3 flex flex-wrap gap-2">
                                            <button onClick={() => id && runAction('Confirming outcome', () => openclaw.reviewLearningOutcome(id, 'confirm', 'positive'))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-300 border border-emerald-500/20">
                                                <ThumbsUp className="w-3 h-3" />
                                                Positive
                                            </button>
                                            <button onClick={() => id && runAction('Confirming outcome', () => openclaw.reviewLearningOutcome(id, 'confirm', 'negative'))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-red-500/10 hover:bg-red-500/20 text-red-300 border border-red-500/20">
                                                <ThumbsDown className="w-3 h-3" />
                                                Negative
                                            </button>
                                            <button onClick={() => id && runAction('Dismissing outcome', () => openclaw.reviewLearningOutcome(id, 'dismiss'))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5">
                                                <X className="w-3 h-3" />
                                                Dismiss
                                            </button>
                                            <button onClick={() => id && runAction('Requeueing outcome', () => openclaw.reviewLearningOutcome(id, 'requeue'))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5">
                                                <RefreshCw className="w-3 h-3" />
                                                Requeue
                                            </button>
                                        </div>
                                    </div>
                                );
                            })}
                        </div>
                    )}
                </Panel>
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-3 gap-5">
                <Panel title="Candidates" icon={Lightbulb}>
                    {candidates.length === 0 ? <Empty text="No learning candidates." /> : (
                        <div className="space-y-2">
                            {candidates.slice(0, 10).map((candidate, index) => (
                                <div key={String(candidate.id || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="text-sm font-semibold truncate">{text(candidate.summary, 'Candidate')}</div>
                                    <div className="text-[11px] text-muted-foreground truncate">{text(candidate.candidate_type)} · {text(candidate.risk_tier)} · {text(candidate.target_name)}</div>
                                </div>
                            ))}
                        </div>
                    )}
                </Panel>

                <Panel title="History" icon={History}>
                    {history.length === 0 ? <Empty text="No learning history." /> : (
                        <div className="space-y-2">
                            {history.slice(0, 10).map((event, index) => (
                                <div key={String(event.id || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="text-sm font-semibold truncate">{text(event.summary, 'Learning event')}</div>
                                    <div className="text-[11px] text-muted-foreground truncate">{text(event.class)} · {text(event.risk_tier)} · {text(event.created_at)}</div>
                                </div>
                            ))}
                            {evaluations.length > 0 && (
                                <div className="pt-2 border-t border-border/40 text-[10px] text-muted-foreground">
                                    {evaluations.length} recent evaluations loaded
                                </div>
                            )}
                        </div>
                    )}
                </Panel>

                <Panel title="Providers" icon={ShieldAlert}>
                    {providers.length === 0 ? <Empty text="No provider health entries." /> : (
                        <div className="space-y-2">
                            {providers.map((provider, index) => (
                                <div key={String(provider.provider || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="flex items-center justify-between gap-3">
                                        <div className="text-sm font-semibold truncate">{text(provider.provider, 'Provider')}</div>
                                        <span className={cn("text-[10px] uppercase font-bold", provider.healthy ? "text-primary" : "text-amber-300")}>{provider.healthy ? 'healthy' : text(provider.readiness)}</span>
                                    </div>
                                    <div className="text-[11px] text-muted-foreground truncate">{provider.enabled ? 'enabled' : 'disabled'} · {provider.active ? 'active' : 'inactive'} · {provider.latency_ms ? `${provider.latency_ms}ms` : 'no latency'}</div>
                                </div>
                            ))}
                        </div>
                    )}
                </Panel>
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-2 gap-5">
                <Panel title="Artifact Versions" icon={FileCode2}>
                    {versions.length === 0 ? <Empty text="No artifact versions." /> : (
                        <div className="space-y-2">
                            {versions.slice(0, 10).map((version, index) => (
                                <div key={String(version.id || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="text-sm font-semibold truncate">{text(version.artifact_name, 'Artifact')}</div>
                                    <div className="text-[11px] text-muted-foreground truncate">{text(version.artifact_type)} · {text(version.version_label)} · {text(version.status)}</div>
                                </div>
                            ))}
                        </div>
                    )}
                </Panel>

                <Panel title="Rollbacks" icon={RotateCcw}>
                    <div className="grid grid-cols-1 md:grid-cols-3 gap-2 mb-3">
                        <input value={rollbackForm.artifactType} onChange={(e) => setRollbackForm(prev => ({ ...prev, artifactType: e.target.value }))} placeholder="artifact type" className="px-3 py-2 rounded-lg bg-background/60 border border-border/40 text-xs outline-none focus:border-primary/40" />
                        <input value={rollbackForm.artifactName} onChange={(e) => setRollbackForm(prev => ({ ...prev, artifactName: e.target.value }))} placeholder="artifact name" className="px-3 py-2 rounded-lg bg-background/60 border border-border/40 text-xs outline-none focus:border-primary/40" />
                        <input value={rollbackForm.reason} onChange={(e) => setRollbackForm(prev => ({ ...prev, reason: e.target.value }))} placeholder="reason" className="px-3 py-2 rounded-lg bg-background/60 border border-border/40 text-xs outline-none focus:border-primary/40" />
                    </div>
                    <button onClick={submitRollback} className="mb-4 inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5">
                        <RotateCcw className="w-3.5 h-3.5" />
                        Record Rollback
                    </button>
                    {rollbacks.length === 0 ? <Empty text="No rollback records." /> : (
                        <div className="space-y-2">
                            {rollbacks.slice(0, 8).map((rollback, index) => (
                                <div key={String(rollback.id || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="text-sm font-semibold truncate">{text(rollback.artifact_name, 'Rollback')}</div>
                                    <div className="text-[11px] text-muted-foreground truncate">{text(rollback.artifact_type)} · {text(rollback.reason)}</div>
                                </div>
                            ))}
                        </div>
                    )}
                </Panel>
            </div>
        </motion.div>
    );
}
