import { useCallback, useEffect, useMemo, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    FolderGit2,
    KeyRound,
    Link2,
    RefreshCw,
    CheckCircle2,
    Circle,
    Loader2,
    ShieldCheck,
    ChevronDown,
    ChevronRight,
    Lock,
    Power,
    Search,
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';

type WriteModeSelection = 'recommended' | thinclaw.ThinClawRepoWriteMode;

/**
 * GitHub connector: connect once (GitHub App install or a personal token),
 * discover the repositories the credential can act on, and select all or
 * specific repos to bring under supervision.
 *
 * Credential VALUES go straight to the encrypted secrets store via
 * {@link thinclaw.setRepoProjectCredential}; they are never written to
 * settings, events, or logs, and the agent/LLM never sees them.
 */
export function ThinClawRepoConnector({ onConnected }: { onConnected?: () => void }) {
    const [readiness, setReadiness] = useState<thinclaw.ThinClawRepoProjectsReadiness | null>(null);
    const [loading, setLoading] = useState(false);
    const [busy, setBusy] = useState<string | null>(null);
    const [expanded, setExpanded] = useState(false);

    const [appForm, setAppForm] = useState({
        app_id: '',
        installation_id: '',
        app_slug: '',
        private_key_secret: '',
    });
    const [cred, setCred] = useState({ name: 'github_token', value: '' });

    const [repos, setRepos] = useState<thinclaw.ThinClawConnectableRepo[]>([]);
    const [reposLoaded, setReposLoaded] = useState(false);
    const [selected, setSelected] = useState<Record<string, boolean>>({});
    const [filter, setFilter] = useState('');
    const [writeMode, setWriteMode] = useState<WriteModeSelection>('recommended');

    const loadReadiness = useCallback(async () => {
        setLoading(true);
        try {
            const result = await thinclaw.getRepoProjectsReadiness();
            setReadiness(result);
            setAppForm((current) => ({
                app_id: result.app_id != null ? String(result.app_id) : current.app_id,
                installation_id:
                    result.installation_id != null ? String(result.installation_id) : current.installation_id,
                app_slug: result.app_slug ?? current.app_slug,
                private_key_secret: result.private_key_secret ?? current.private_key_secret,
            }));
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        loadReadiness();
    }, [loadReadiness]);

    const enabled = readiness?.enabled ?? false;
    const credentialMode = readiness?.credential_mode ?? 'none';
    const ready = readiness?.ready_for_live_runs ?? false;

    const toggleEnabled = async () => {
        setBusy('enable');
        try {
            const result = await thinclaw.setupRepoProjects({ enabled: !enabled });
            setReadiness(result);
            if (!result.unavailable) {
                toast.success(!enabled ? 'Supervisor enabled' : 'Supervisor disabled');
                setExpanded(true);
            } else {
                toast.error(result.unavailable.reason);
            }
        } catch (error) {
            toast.error(String(error));
        } finally {
            setBusy(null);
        }
    };

    const saveApp = async () => {
        setBusy('app');
        try {
            const result = await thinclaw.setupRepoProjects({
                app_id: appForm.app_id ? Number(appForm.app_id) : undefined,
                installation_id: appForm.installation_id ? Number(appForm.installation_id) : undefined,
                app_slug: appForm.app_slug || undefined,
                private_key_secret: appForm.private_key_secret || undefined,
            });
            setReadiness(result);
            result.unavailable
                ? toast.error(result.unavailable.reason)
                : toast.success('GitHub App configuration saved');
        } catch (error) {
            toast.error(String(error));
        } finally {
            setBusy(null);
        }
    };

    const saveCredential = async () => {
        const name = cred.name.trim() || 'github_token';
        if (!cred.value.trim()) {
            toast.error('Enter the credential value');
            return;
        }
        setBusy('cred');
        try {
            const result = await thinclaw.setRepoProjectCredential(name, cred.value);
            if (result.unavailable) {
                toast.error(result.unavailable.reason);
                return;
            }
            setCred((current) => ({ ...current, value: '' }));
            toast.success(`Stored ${name} securely`);
            await loadReadiness();
        } catch (error) {
            toast.error(String(error));
        } finally {
            setBusy(null);
        }
    };

    const discover = async () => {
        setBusy('discover');
        try {
            const result = await thinclaw.listConnectableRepos();
            if (result.unavailable) {
                toast.error(result.unavailable.reason);
                return;
            }
            setRepos(result.repos);
            setReposLoaded(true);
            setSelected({});
            if (result.repos.length === 0) {
                toast.message('No repositories found for the connected credential');
            }
        } catch (error) {
            toast.error(String(error));
        } finally {
            setBusy(null);
        }
    };

    const connect = async (all: boolean) => {
        const picks = all ? [] : Object.keys(selected).filter((key) => selected[key]);
        if (!all && picks.length === 0) {
            toast.error('Select at least one repository');
            return;
        }
        setBusy('connect');
        try {
            const input: thinclaw.ThinClawRepoConnectInput = all ? { all: true } : { repos: picks };
            if (writeMode !== 'recommended') {
                input.write_mode = writeMode;
            }
            const result = await thinclaw.connectRepoProjects(input);
            if (result.unavailable) {
                toast.error(result.unavailable.reason);
                return;
            }
            if (result.ok) {
                toast.success(result.message);
                onConnected?.();
                await discover();
            } else {
                toast.error(result.message);
            }
        } catch (error) {
            toast.error(String(error));
        } finally {
            setBusy(null);
        }
    };

    const visibleRepos = useMemo(() => {
        const needle = filter.trim().toLowerCase();
        if (!needle) return repos;
        return repos.filter((repo) => repo.full_name.toLowerCase().includes(needle));
    }, [repos, filter]);

    const selectableCount = visibleRepos.filter((repo) => !repo.enrolled).length;
    const selectedCount = Object.values(selected).filter(Boolean).length;

    const credentialTone =
        credentialMode === 'none'
            ? 'border-amber-500/30 bg-amber-500/10 text-amber-200'
            : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';

    return (
        <div className="rounded-lg border border-border/40 bg-card/30 overflow-hidden">
            {/* Header */}
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/40 px-4 py-3">
                <button
                    onClick={() => setExpanded((value) => !value)}
                    className="flex items-center gap-2.5 text-left"
                >
                    {expanded ? (
                        <ChevronDown className="h-4 w-4 text-muted-foreground" />
                    ) : (
                        <ChevronRight className="h-4 w-4 text-muted-foreground" />
                    )}
                    <div className="rounded-md border border-white/10 bg-white/4 p-1.5">
                        <FolderGit2 className="h-4 w-4 text-foreground" />
                    </div>
                    <div>
                        <p className="text-sm font-semibold">GitHub Connector</p>
                        <p className="text-[10px] text-muted-foreground">
                            Connect GitHub and choose which repositories the agent may manage
                        </p>
                    </div>
                </button>
                <div className="flex items-center gap-2">
                    <span className={cn('rounded-full border px-2 py-0.5 text-[10px] font-medium', credentialTone)}>
                        {credentialMode === 'github_app'
                            ? 'GitHub App'
                            : credentialMode === 'github_token'
                              ? 'Token'
                              : 'No credentials'}
                    </span>
                    {ready && (
                        <span className="flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-200">
                            <ShieldCheck className="h-3 w-3" /> Live-ready
                        </span>
                    )}
                    <button
                        onClick={toggleEnabled}
                        disabled={busy === 'enable'}
                        className={cn(
                            'flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-[11px] font-medium transition-colors disabled:opacity-50',
                            enabled
                                ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200 hover:bg-emerald-500/20'
                                : 'border-white/10 bg-white/3 text-muted-foreground hover:bg-white/5 hover:text-foreground',
                        )}
                        title={enabled ? 'Disable supervisor' : 'Enable supervisor'}
                    >
                        {busy === 'enable' ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        ) : (
                            <Power className="h-3.5 w-3.5" />
                        )}
                        {enabled ? 'Enabled' : 'Enable'}
                    </button>
                    <button
                        onClick={loadReadiness}
                        className="rounded-lg border border-white/5 bg-white/3 p-1.5 text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground"
                        title="Refresh connector status"
                    >
                        <RefreshCw className={cn('h-3.5 w-3.5', loading && 'animate-spin')} />
                    </button>
                </div>
            </div>

            <AnimatePresence initial={false}>
                {expanded && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="space-y-5 p-4">
                            {/* Readiness checklist */}
                            {readiness?.checklist?.length ? (
                                <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
                                    {readiness.checklist.map((item) => {
                                        const done = item.state === 'complete' || item.state === 'enabled';
                                        return (
                                            <div
                                                key={item.key}
                                                className="flex items-start gap-2 rounded-md border border-white/5 bg-black/20 px-2.5 py-2"
                                            >
                                                {done ? (
                                                    <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-emerald-400" />
                                                ) : (
                                                    <Circle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                                                )}
                                                <div className="min-w-0">
                                                    <p className="text-[11px] font-medium">{item.label}</p>
                                                    {item.detail && (
                                                        <p className="truncate text-[10px] text-muted-foreground">{item.detail}</p>
                                                    )}
                                                </div>
                                            </div>
                                        );
                                    })}
                                </div>
                            ) : null}

                            <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
                                {/* Credential entry (secure) */}
                                <section className="space-y-2.5">
                                    <div className="flex items-center gap-2">
                                        <Lock className="h-3.5 w-3.5 text-primary" />
                                        <p className="text-[11px] font-bold uppercase tracking-widest text-muted-foreground">
                                            Store credential
                                        </p>
                                    </div>
                                    <p className="text-[10px] text-muted-foreground">
                                        Stored encrypted in the secrets store. The value never reaches settings, logs, or
                                        the model. Use <span className="font-mono">github_token</span> for a PAT, or a
                                        name like <span className="font-mono">repo_projects_github_private_key</span> for
                                        a GitHub App PEM key.
                                    </p>
                                    <input
                                        value={cred.name}
                                        onChange={(event) => setCred((current) => ({ ...current, name: event.target.value }))}
                                        placeholder="Secret name (e.g. github_token)"
                                        className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs font-mono outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                    />
                                    <input
                                        type="password"
                                        value={cred.value}
                                        onChange={(event) => setCred((current) => ({ ...current, value: event.target.value }))}
                                        placeholder="Paste token or PEM key — stored encrypted"
                                        autoComplete="off"
                                        className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                    />
                                    <button
                                        onClick={saveCredential}
                                        disabled={busy === 'cred'}
                                        className="flex w-full items-center justify-center gap-1.5 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs font-medium text-primary transition-colors hover:bg-primary/20 disabled:opacity-50"
                                    >
                                        {busy === 'cred' ? (
                                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                        ) : (
                                            <KeyRound className="h-3.5 w-3.5" />
                                        )}
                                        Store securely
                                    </button>
                                </section>

                                {/* GitHub App config */}
                                <section className="space-y-2.5">
                                    <div className="flex items-center gap-2">
                                        <FolderGit2 className="h-3.5 w-3.5 text-primary" />
                                        <p className="text-[11px] font-bold uppercase tracking-widest text-muted-foreground">
                                            GitHub App (optional)
                                        </p>
                                    </div>
                                    <p className="text-[10px] text-muted-foreground">
                                        Configure a GitHub App to use installation tokens and native repo selection. Leave
                                        blank to use a personal token instead.
                                    </p>
                                    <div className="grid grid-cols-2 gap-2">
                                        <input
                                            value={appForm.app_id}
                                            onChange={(event) => setAppForm((current) => ({ ...current, app_id: event.target.value }))}
                                            placeholder="App ID"
                                            inputMode="numeric"
                                            className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                        />
                                        <input
                                            value={appForm.installation_id}
                                            onChange={(event) =>
                                                setAppForm((current) => ({ ...current, installation_id: event.target.value }))
                                            }
                                            placeholder="Installation ID"
                                            inputMode="numeric"
                                            className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                        />
                                    </div>
                                    <input
                                        value={appForm.app_slug}
                                        onChange={(event) => setAppForm((current) => ({ ...current, app_slug: event.target.value }))}
                                        placeholder="App slug (for the install link)"
                                        className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                    />
                                    <input
                                        value={appForm.private_key_secret}
                                        onChange={(event) =>
                                            setAppForm((current) => ({ ...current, private_key_secret: event.target.value }))
                                        }
                                        placeholder="Private key secret name"
                                        className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs font-mono outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                    />
                                    <div className="flex items-center gap-2">
                                        <button
                                            onClick={saveApp}
                                            disabled={busy === 'app'}
                                            className="flex flex-1 items-center justify-center gap-1.5 rounded-lg border border-white/10 bg-white/3 px-3 py-2 text-xs font-medium transition-colors hover:bg-white/5 disabled:opacity-50"
                                        >
                                            {busy === 'app' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                                            Save App config
                                        </button>
                                        {readiness?.install_url && (
                                            <a
                                                href={readiness.install_url}
                                                target="_blank"
                                                rel="noreferrer"
                                                className="flex items-center justify-center gap-1.5 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs font-medium text-primary transition-colors hover:bg-primary/20"
                                                title={readiness.install_url}
                                            >
                                                <Link2 className="h-3.5 w-3.5" /> Install
                                            </a>
                                        )}
                                    </div>
                                </section>
                            </div>

                            {/* Repo picker */}
                            <section className="space-y-2.5">
                                <div className="flex flex-wrap items-center justify-between gap-2">
                                    <p className="text-[11px] font-bold uppercase tracking-widest text-muted-foreground">
                                        Select repositories
                                    </p>
                                    <div className="flex items-center gap-2">
                                        <button
                                            onClick={discover}
                                            disabled={busy === 'discover'}
                                            className="flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/3 px-2.5 py-1 text-[11px] font-medium transition-colors hover:bg-white/5 disabled:opacity-50"
                                        >
                                            {busy === 'discover' ? (
                                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                            ) : (
                                                <Search className="h-3.5 w-3.5" />
                                            )}
                                            Discover repos
                                        </button>
                                        <button
                                            onClick={() => connect(true)}
                                            disabled={busy === 'connect' || !enabled}
                                            className="flex items-center gap-1.5 rounded-lg border border-primary/30 bg-primary/10 px-2.5 py-1 text-[11px] font-medium text-primary transition-colors hover:bg-primary/20 disabled:opacity-50"
                                            title={enabled ? 'Connect every accessible repository' : 'Enable the supervisor first'}
                                        >
                                            Connect all
                                        </button>
                                    </div>
                                </div>

                                <div className="flex flex-wrap items-center gap-2">
                                    <select
                                        value={writeMode}
                                        onChange={(event) => setWriteMode(event.target.value as WriteModeSelection)}
                                        className="rounded-lg border border-white/5 bg-black/20 px-2.5 py-1.5 text-[11px] outline-none focus:border-primary/40"
                                    >
                                        <option value="recommended">Recommended per repo</option>
                                        <option value="read_only_clone">Read-only clone</option>
                                        <option value="fork_pr">Fork PR</option>
                                        <option value="maintainer_branch_pr">Maintainer branch PR</option>
                                        <option value="maintainer_auto_merge">Maintainer auto-merge</option>
                                    </select>
                                    <span className="text-[10px] text-muted-foreground">
                                        Default: {readiness?.default_write_mode ?? 'fork_pr'}
                                    </span>
                                </div>

                                {reposLoaded && repos.length > 0 && (
                                    <input
                                        value={filter}
                                        onChange={(event) => setFilter(event.target.value)}
                                        placeholder="Filter repositories…"
                                        className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                    />
                                )}

                                <div className="max-h-[280px] overflow-y-auto rounded-lg border border-white/5 bg-black/20">
                                    {!reposLoaded ? (
                                        <p className="px-3 py-6 text-center text-[11px] text-muted-foreground">
                                            Store a credential, then “Discover repos” to list what the agent can manage.
                                        </p>
                                    ) : visibleRepos.length === 0 ? (
                                        <p className="px-3 py-6 text-center text-[11px] text-muted-foreground">
                                            No repositories match.
                                        </p>
                                    ) : (
                                        visibleRepos.map((repo) => {
                                            const key = repo.full_name;
                                            const checked = !!selected[key];
                                            return (
                                                <label
                                                    key={key}
                                                    className={cn(
                                                        'flex cursor-pointer items-center gap-3 border-b border-border/20 px-3 py-2 last:border-b-0 transition-colors hover:bg-white/3',
                                                        repo.enrolled && 'cursor-default opacity-60 hover:bg-transparent',
                                                    )}
                                                >
                                                    <input
                                                        type="checkbox"
                                                        disabled={repo.enrolled || busy === 'connect'}
                                                        checked={checked}
                                                        onChange={(event) =>
                                                            setSelected((current) => ({ ...current, [key]: event.target.checked }))
                                                        }
                                                        className="h-3.5 w-3.5 accent-primary"
                                                    />
                                                    <div className="min-w-0 flex-1">
                                                        <p className="truncate text-xs font-medium font-mono">{repo.full_name}</p>
                                                        <p className="text-[10px] text-muted-foreground">
                                                            {repo.private ? 'private' : 'public'} · {repo.default_branch}
                                                            {repo.archived ? ' · archived' : ''}
                                                            {' · '}
                                                            {repo.recommended_write_mode}
                                                        </p>
                                                    </div>
                                                    {repo.enrolled && (
                                                        <span className="flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[9px] font-medium text-emerald-200">
                                                            <CheckCircle2 className="h-2.5 w-2.5" /> Supervised
                                                        </span>
                                                    )}
                                                </label>
                                            );
                                        })
                                    )}
                                </div>

                                {reposLoaded && (
                                    <div className="flex items-center justify-between">
                                        <span className="text-[10px] text-muted-foreground">
                                            {selectableCount} available · {selectedCount} selected
                                        </span>
                                        <button
                                            onClick={() => connect(false)}
                                            disabled={busy === 'connect' || selectedCount === 0 || !enabled}
                                            className="flex items-center gap-1.5 rounded-lg border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs font-medium text-primary transition-colors hover:bg-primary/20 disabled:opacity-50"
                                        >
                                            {busy === 'connect' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                                            Connect selected
                                        </button>
                                    </div>
                                )}
                            </section>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
