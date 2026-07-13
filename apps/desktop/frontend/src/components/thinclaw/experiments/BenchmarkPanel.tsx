import { useEffect, useMemo, useState } from 'react';
import { AlertTriangle, CheckCircle2, FlaskConical, Loader2, Play } from 'lucide-react';
import { toast } from 'sonner';

import { cn } from '../../../lib/utils';
import {
    getExperimentEnvironments,
    runExperimentEvaluation,
    type ExperimentEnvironment,
    type ExperimentEvalResult,
} from '../../../lib/thinclaw';

const DEFAULT_PROMPT = 'Briefly summarize your current capabilities.';

function boundedInteger(value: string, fallback: number, min: number, max: number): number {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? Math.min(max, Math.max(min, parsed)) : fallback;
}

function resultPercent(result: ExperimentEvalResult): number {
    const score = Number(result.summary?.score);
    return Number.isFinite(score) ? Math.round(Math.min(1, Math.max(0, score)) * 100) : 0;
}

export function BenchmarkPanel() {
    const [environments, setEnvironments] = useState<ExperimentEnvironment[]>([]);
    const [selectedId, setSelectedId] = useState('agent_loop');
    const [prompt, setPrompt] = useState(DEFAULT_PROMPT);
    const [episodes, setEpisodes] = useState(1);
    const [maxSteps, setMaxSteps] = useState(4);
    const [loading, setLoading] = useState(true);
    const [running, setRunning] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [result, setResult] = useState<ExperimentEvalResult | null>(null);

    useEffect(() => {
        let cancelled = false;
        getExperimentEnvironments()
            .then((catalog) => {
                if (cancelled) return;
                const available = Array.isArray(catalog.environments) ? catalog.environments : [];
                setEnvironments(available);
                const firstRunnable = available.find((environment) => environment.runnable);
                if (firstRunnable) setSelectedId(firstRunnable.id);
            })
            .catch((cause) => {
                if (!cancelled) setError(String(cause));
            })
            .finally(() => {
                if (!cancelled) setLoading(false);
            });
        return () => {
            cancelled = true;
        };
    }, []);

    const selected = useMemo(
        () => environments.find((environment) => environment.id === selectedId) ?? null,
        [environments, selectedId],
    );

    const run = async () => {
        if (!selected?.runnable || running) return;
        setRunning(true);
        setError(null);
        setResult(null);
        try {
            const evaluation = await runExperimentEvaluation(
                selected.id,
                prompt.trim() || DEFAULT_PROMPT,
                episodes,
                maxSteps,
            );
            setResult(evaluation);
            toast.success(`Completed ${evaluation.episodes} evaluation ${evaluation.episodes === 1 ? 'episode' : 'episodes'}`);
        } catch (cause) {
            const message = String(cause);
            setError(message);
            toast.error(message);
        } finally {
            setRunning(false);
        }
    };

    return (
        <section className="rounded-xl border border-border/40 bg-card/30 p-5" aria-labelledby="benchmarks-title">
            <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                    <div className="flex items-center gap-2">
                        <FlaskConical className="h-4 w-4 text-primary" />
                        <h2 id="benchmarks-title" className="text-sm font-bold">Benchmarks</h2>
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                        Run bounded evaluations against the embedded agent in isolated, throwaway sessions.
                    </p>
                </div>
                <span className="rounded-md border border-border/50 bg-background/30 px-2 py-1 text-[10px] font-bold uppercase tracking-wide text-muted-foreground">
                    Local runtime only
                </span>
            </div>

            {loading ? (
                <div className="flex items-center gap-2 py-8 text-xs text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Loading evaluator environments…
                </div>
            ) : (
                <div className="mt-5 grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(300px,0.8fr)]">
                    <div className="space-y-4">
                        <div className="grid gap-2 sm:grid-cols-3" role="radiogroup" aria-label="Evaluator environment">
                            {environments.map((environment) => {
                                const active = selectedId === environment.id;
                                return (
                                    <button
                                        key={environment.id}
                                        type="button"
                                        role="radio"
                                        aria-checked={active}
                                        disabled={!environment.runnable}
                                        onClick={() => setSelectedId(environment.id)}
                                        className={cn(
                                            'rounded-lg border p-3 text-left transition-colors',
                                            active ? 'border-primary/40 bg-primary/5' : 'border-border/40 bg-background/20',
                                            environment.runnable ? 'hover:bg-muted/30' : 'cursor-not-allowed opacity-55',
                                        )}
                                    >
                                        <div className="flex items-center justify-between gap-2">
                                            <span className="text-xs font-semibold">{environment.name}</span>
                                            <span className="text-[9px] font-bold uppercase text-muted-foreground">
                                                {environment.runnable ? 'Ready' : 'CLI cases'}
                                            </span>
                                        </div>
                                        <p className="mt-1.5 text-[10px] leading-relaxed text-muted-foreground">
                                            {environment.description}
                                        </p>
                                    </button>
                                );
                            })}
                        </div>

                        {environments.length === 0 && (
                            <div className="rounded-lg border border-dashed border-border/40 p-4 text-xs text-muted-foreground">
                                No evaluator environments were reported by the runtime.
                            </div>
                        )}

                        <label className="block space-y-1.5 text-xs font-medium">
                            Evaluation prompt
                            <textarea
                                value={prompt}
                                onChange={(event) => setPrompt(event.target.value)}
                                rows={3}
                                className="w-full resize-y rounded-lg border border-border/50 bg-background/40 px-3 py-2 text-xs font-normal outline-none focus:border-primary/50"
                            />
                        </label>

                        <div className="grid grid-cols-2 gap-3">
                            <label className="space-y-1.5 text-xs font-medium">
                                Episodes
                                <input
                                    aria-label="Episodes"
                                    type="number"
                                    min={1}
                                    max={20}
                                    value={episodes}
                                    onChange={(event) => setEpisodes(boundedInteger(event.target.value, 1, 1, 20))}
                                    className="w-full rounded-lg border border-border/50 bg-background/40 px-3 py-2 text-xs font-normal outline-none focus:border-primary/50"
                                />
                            </label>
                            <label className="space-y-1.5 text-xs font-medium">
                                Max steps
                                <input
                                    aria-label="Max steps"
                                    type="number"
                                    min={1}
                                    max={16}
                                    value={maxSteps}
                                    onChange={(event) => setMaxSteps(boundedInteger(event.target.value, 4, 1, 16))}
                                    className="w-full rounded-lg border border-border/50 bg-background/40 px-3 py-2 text-xs font-normal outline-none focus:border-primary/50"
                                />
                            </label>
                        </div>

                        <button
                            type="button"
                            onClick={run}
                            disabled={!selected?.runnable || running}
                            className="inline-flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs font-semibold text-primary transition-colors hover:bg-primary/15 disabled:cursor-not-allowed disabled:opacity-50"
                        >
                            {running ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                            {running ? 'Running evaluation…' : 'Run benchmark'}
                        </button>
                    </div>

                    <div className="rounded-xl border border-border/40 bg-background/20 p-4" aria-live="polite">
                        <div className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground">Latest result</div>
                        {result ? (
                            <div className="mt-4 space-y-4">
                                <div className="flex items-center gap-3">
                                    <CheckCircle2 className="h-5 w-5 text-emerald-400" />
                                    <div>
                                        <div className="text-2xl font-bold tabular-nums">{resultPercent(result)}% score</div>
                                        <div className="text-[11px] text-muted-foreground">{result.env_id.split('_').join(' ')}</div>
                                    </div>
                                </div>
                                <dl className="grid grid-cols-2 gap-2 text-xs">
                                    <div className="rounded-lg border border-border/30 p-3">
                                        <dt className="text-[10px] uppercase text-muted-foreground">Episodes</dt>
                                        <dd className="mt-1 font-semibold tabular-nums">{result.summary.episode_count}</dd>
                                    </div>
                                    <div className="rounded-lg border border-border/30 p-3">
                                        <dt className="text-[10px] uppercase text-muted-foreground">Steps</dt>
                                        <dd className="mt-1 font-semibold tabular-nums">{result.summary.step_count}</dd>
                                    </div>
                                    <div className="rounded-lg border border-border/30 p-3">
                                        <dt className="text-[10px] uppercase text-muted-foreground">Token captures</dt>
                                        <dd className="mt-1 font-semibold tabular-nums">{result.summary.token_capture_steps}</dd>
                                    </div>
                                    <div className="rounded-lg border border-border/30 p-3">
                                        <dt className="text-[10px] uppercase text-muted-foreground">Logprobs</dt>
                                        <dd className="mt-1 font-semibold">{result.summary.logprobs_supported ? 'Available' : 'Unavailable'}</dd>
                                    </div>
                                </dl>
                            </div>
                        ) : (
                            <p className="mt-4 text-xs leading-relaxed text-muted-foreground">
                                Choose a runnable environment and start an evaluation. Results stay in this view; your normal chat threads are not modified.
                            </p>
                        )}

                        {error && (
                            <div className="mt-4 flex items-start gap-2 rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-300" role="alert">
                                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                                <span>{error}</span>
                            </div>
                        )}
                    </div>
                </div>
            )}
        </section>
    );
}
