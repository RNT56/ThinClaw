import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { motion } from 'framer-motion';
import {
    AlertTriangle, Beaker, Boxes, Cloud, Cpu, FlaskConical, Gauge,
    GitBranch, Play, RefreshCw, RotateCw, ShieldCheck, Target, XCircle
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';

type JsonMap = Record<string, any>;

function arrayFrom(data: unknown, key: string): JsonMap[] {
    const value = (data as JsonMap | null)?.[key];
    return Array.isArray(value) ? value as JsonMap[] : [];
}

function unavailableReason(data: unknown): string | null {
    const value = data as JsonMap | null;
    return value?.available === false ? String(value.reason || 'Unavailable') : null;
}

function itemTitle(item: JsonMap, fallback: string) {
    return String(item.name || item.title || item.id || fallback);
}

function statusText(item: JsonMap) {
    const value = item.status || item.readiness_class || item.queue_state || item.launch_eligible;
    if (typeof value === 'boolean') return value ? 'ready' : 'not ready';
    return value ? String(value) : 'unknown';
}

function Stat({ icon: Icon, label, value }: { icon: any; label: string; value: string | number }) {
    return (
        <div className="rounded-xl border border-border/40 bg-card/30 p-4">
            <div className="flex items-center gap-2 text-[10px] uppercase font-bold tracking-widest text-muted-foreground">
                <Icon className="w-3.5 h-3.5 text-primary" />
                {label}
            </div>
            <div className="mt-2 text-2xl font-bold tabular-nums">{value}</div>
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

export function ThinClawExperiments() {
    const [loading, setLoading] = useState(true);
    const [refreshing, setRefreshing] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [data, setData] = useState<Record<string, unknown>>({});
    const [selectedCampaign, setSelectedCampaign] = useState<string | null>(null);
    const [selectedTrial, setSelectedTrial] = useState<string | null>(null);

    const load = useCallback(async () => {
        setRefreshing(true);
        setError(null);
        try {
            const [
                projects,
                campaigns,
                runners,
                targets,
                usage,
                opportunities,
                gpuClouds,
            ] = await Promise.all([
                thinclaw.getExperimentProjects(),
                thinclaw.getExperimentCampaigns(),
                thinclaw.getExperimentRunners(),
                thinclaw.getExperimentTargets(),
                thinclaw.getExperimentModelUsage(100),
                thinclaw.getExperimentOpportunities(100),
                thinclaw.getExperimentGpuClouds(),
            ]);
            setData({ projects, campaigns, runners, targets, usage, opportunities, gpuClouds });
            const firstCampaign = arrayFrom(campaigns, 'campaigns')[0]?.id;
            setSelectedCampaign(current => current || (firstCampaign ? String(firstCampaign) : null));
        } catch (err) {
            setError(String(err));
        } finally {
            setLoading(false);
            setRefreshing(false);
        }
    }, []);

    useEffect(() => {
        load();
    }, [load]);

    useEffect(() => {
        if (!selectedCampaign) return;
        thinclaw.getExperimentTrials(selectedCampaign)
            .then((trials) => {
                setData(prev => ({ ...prev, trials }));
                const firstTrial = arrayFrom(trials, 'trials')[0]?.id;
                setSelectedTrial(firstTrial ? String(firstTrial) : null);
            })
            .catch((err) => setData(prev => ({ ...prev, trials: { available: false, reason: String(err) } })));
    }, [selectedCampaign]);

    useEffect(() => {
        if (!selectedTrial) return;
        thinclaw.getExperimentTrialArtifacts(selectedTrial)
            .then((artifacts) => setData(prev => ({ ...prev, artifacts })))
            .catch((err) => setData(prev => ({ ...prev, artifacts: { available: false, reason: String(err) } })));
    }, [selectedTrial]);

    const projects = arrayFrom(data.projects, 'projects');
    const campaigns = arrayFrom(data.campaigns, 'campaigns');
    const runners = arrayFrom(data.runners, 'runners');
    const targets = arrayFrom(data.targets, 'targets');
    const trials = arrayFrom(data.trials, 'trials');
    const usage = arrayFrom(data.usage, 'usage');
    const opportunities = arrayFrom(data.opportunities, 'opportunities');
    const providers = arrayFrom(data.gpuClouds, 'providers');
    const artifacts = arrayFrom(data.artifacts, 'artifacts');
    const unavailable = useMemo(() => [
        unavailableReason(data.projects),
        unavailableReason(data.campaigns),
        unavailableReason(data.runners),
        unavailableReason(data.targets),
        unavailableReason(data.usage),
        unavailableReason(data.opportunities),
        unavailableReason(data.gpuClouds),
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
                    <div className="p-2.5 rounded-xl bg-emerald-500/10 border border-emerald-500/20">
                        <FlaskConical className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Experiments</h1>
                        <p className="text-xs text-muted-foreground">Research campaigns, runners, targets, usage, opportunities, and GPU cloud readiness</p>
                    </div>
                </div>
                <button
                    onClick={load}
                    className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                    title="Refresh experiments"
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
                <Stat icon={Beaker} label="Projects" value={projects.length} />
                <Stat icon={GitBranch} label="Campaigns" value={campaigns.length} />
                <Stat icon={Cpu} label="Runners" value={runners.length} />
                <Stat icon={Target} label="Targets" value={targets.length} />
                <Stat icon={Gauge} label="Usage Rows" value={usage.length} />
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-2 gap-5">
                <Panel title="Projects" icon={Beaker}>
                    {projects.length === 0 ? <Empty text="No experiment projects." /> : (
                        <div className="space-y-2">
                            {projects.map((project, index) => (
                                <div key={String(project.id || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="flex items-center justify-between gap-3">
                                        <div className="min-w-0">
                                            <div className="text-sm font-semibold truncate">{itemTitle(project, 'Project')}</div>
                                            <div className="text-[11px] text-muted-foreground truncate">{project.workspace_path || project.base_branch || project.preset || 'No workspace metadata'}</div>
                                        </div>
                                        <span className="text-[10px] uppercase font-bold text-muted-foreground">{statusText(project)}</span>
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}
                </Panel>

                <Panel title="Campaigns" icon={GitBranch}>
                    {campaigns.length === 0 ? <Empty text="No experiment campaigns." /> : (
                        <div className="space-y-2">
                            {campaigns.map((campaign, index) => {
                                const id = String(campaign.id || '');
                                const active = selectedCampaign === id;
                                return (
                                    <button
                                        key={id || index}
                                        onClick={() => id && setSelectedCampaign(id)}
                                        className={cn("w-full text-left rounded-lg border p-3 transition-all", active ? "border-primary/40 bg-primary/5" : "border-border/40 hover:bg-muted/30")}
                                    >
                                        <div className="flex items-center justify-between gap-3">
                                            <div className="min-w-0">
                                                <div className="text-sm font-semibold truncate">{itemTitle(campaign, 'Campaign')}</div>
                                                <div className="text-[11px] text-muted-foreground truncate">{campaign.summary || campaign.pause_reason || campaign.worktree_path || 'No campaign detail'}</div>
                                            </div>
                                            <span className="text-[10px] uppercase font-bold text-muted-foreground">{statusText(campaign)}</span>
                                        </div>
                                        <div className="mt-3 flex flex-wrap gap-2">
                                            {(['pause', 'resume', 'cancel', 'promote', 'reissue-lease'] as const).map(action => (
                                                <span
                                                    key={action}
                                                    onClick={(event) => {
                                                        event.stopPropagation();
                                                        if (id) runAction(`${action} campaign`, () => thinclaw.runExperimentCampaignAction(id, action));
                                                    }}
                                                    className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5"
                                                >
                                                    {action === 'cancel' ? <XCircle className="w-3 h-3" /> : action === 'resume' ? <Play className="w-3 h-3" /> : <RotateCw className="w-3 h-3" />}
                                                    {action}
                                                </span>
                                            ))}
                                        </div>
                                    </button>
                                );
                            })}
                        </div>
                    )}
                </Panel>

                <Panel title="Runners" icon={Cpu}>
                    {runners.length === 0 ? <Empty text="No runner profiles." /> : (
                        <div className="space-y-2">
                            {runners.map((runner, index) => {
                                const id = String(runner.id || '');
                                return (
                                    <div key={id || index} className="rounded-lg border border-border/40 p-3">
                                        <div className="flex items-center justify-between gap-3">
                                            <div className="min-w-0">
                                                <div className="text-sm font-semibold truncate">{itemTitle(runner, 'Runner')}</div>
                                                <div className="text-[11px] text-muted-foreground truncate">{runner.backend || runner.image_or_runtime || 'No runtime metadata'}</div>
                                            </div>
                                            <button
                                                onClick={() => id && runAction('Validating runner', () => thinclaw.validateExperimentRunner(id))}
                                                className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5"
                                            >
                                                <ShieldCheck className="w-3 h-3" />
                                                Validate
                                            </button>
                                        </div>
                                        <div className="mt-2 text-[10px] uppercase font-bold text-muted-foreground">{statusText(runner)}</div>
                                    </div>
                                );
                            })}
                        </div>
                    )}
                </Panel>

                <Panel title="GPU Clouds" icon={Cloud}>
                    {providers.length === 0 ? <Empty text="No GPU cloud providers reported." /> : (
                        <div className="space-y-2">
                            {providers.map((provider, index) => {
                                const slug = String(provider.slug || provider.backend || '');
                                return (
                                    <div key={slug || index} className="rounded-lg border border-border/40 p-3">
                                        <div className="flex items-center justify-between gap-3">
                                            <div className="min-w-0">
                                                <div className="text-sm font-semibold truncate">{provider.display_name || slug}</div>
                                                <div className="text-[11px] text-muted-foreground truncate">{provider.secret_name || provider.docs_url || 'No credential metadata'}</div>
                                            </div>
                                            <span className={cn("text-[10px] uppercase font-bold", provider.connected ? "text-primary" : "text-muted-foreground")}>
                                                {provider.connected ? 'connected' : 'not connected'}
                                            </span>
                                        </div>
                                        <div className="mt-3 flex gap-2">
                                            <button onClick={() => slug && runAction('Validating GPU cloud', () => thinclaw.validateExperimentGpuCloud(slug))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5">
                                                <ShieldCheck className="w-3 h-3" />
                                                Validate
                                            </button>
                                            <button onClick={() => slug && runAction('Launching test', () => thinclaw.launchExperimentGpuCloudTest(slug))} className="inline-flex items-center gap-1.5 px-2 py-1 rounded-md text-[10px] font-medium bg-white/[0.03] hover:bg-white/[0.06] border border-white/5">
                                                <Play className="w-3 h-3" />
                                                Test
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
                <Panel title="Targets" icon={Target}>
                    {targets.length === 0 ? <Empty text="No experiment targets." /> : (
                        <div className="space-y-2">
                            {targets.slice(0, 10).map((target, index) => (
                                <div key={String(target.id || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="text-sm font-semibold truncate">{itemTitle(target, 'Target')}</div>
                                    <div className="text-[11px] text-muted-foreground truncate">{target.kind || target.provider || target.model || 'No target metadata'}</div>
                                </div>
                            ))}
                        </div>
                    )}
                </Panel>
                <Panel title="Opportunities" icon={Boxes}>
                    {opportunities.length === 0 ? <Empty text="No experiment opportunities." /> : (
                        <div className="space-y-2">
                            {opportunities.slice(0, 10).map((opportunity, index) => (
                                <div key={String(opportunity.id || index)} className="rounded-lg border border-border/40 p-3">
                                    <div className="text-sm font-semibold truncate">{opportunity.summary || itemTitle(opportunity, 'Opportunity')}</div>
                                    <div className="text-[11px] text-muted-foreground truncate">{opportunity.provider || opportunity.model || opportunity.kind || 'No opportunity metadata'}</div>
                                </div>
                            ))}
                        </div>
                    )}
                </Panel>
                <Panel title="Trials & Artifacts" icon={FlaskConical}>
                    {trials.length === 0 ? <Empty text="No trials for the selected campaign." /> : (
                        <div className="space-y-2">
                            {trials.slice(0, 8).map((trial, index) => {
                                const id = String(trial.id || '');
                                return (
                                    <button key={id || index} onClick={() => id && setSelectedTrial(id)} className={cn("w-full text-left rounded-lg border p-3", selectedTrial === id ? "border-primary/40 bg-primary/5" : "border-border/40 hover:bg-muted/30")}>
                                        <div className="flex items-center justify-between gap-3">
                                            <div className="text-sm font-semibold truncate">{trial.summary || `Trial ${trial.sequence || index + 1}`}</div>
                                            <span className="text-[10px] uppercase font-bold text-muted-foreground">{statusText(trial)}</span>
                                        </div>
                                    </button>
                                );
                            })}
                            {artifacts.length > 0 && (
                                <div className="pt-2 border-t border-border/40">
                                    <div className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground mb-2">Artifacts</div>
                                    {artifacts.slice(0, 5).map((artifact, index) => (
                                        <div key={String(artifact.id || artifact.path || index)} className="text-xs text-muted-foreground truncate py-1">{artifact.path || artifact.name || artifact.kind || 'Artifact'}</div>
                                    ))}
                                </div>
                            )}
                        </div>
                    )}
                </Panel>
            </div>
        </motion.div>
    );
}
