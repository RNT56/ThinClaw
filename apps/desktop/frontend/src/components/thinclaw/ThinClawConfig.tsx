import { useCallback, useEffect, useMemo, useState } from 'react';
import { Copy, RefreshCw, Search, Settings2 } from 'lucide-react';
import { toast } from 'sonner';

import * as thinclaw from '../../lib/thinclaw';
import { AsyncState, Button, Notice, Surface } from '../ui';

interface SettingEntry {
    key: string;
    value: unknown;
    updated_at: string;
}

function isSensitiveSetting(key: string): boolean {
    return /(secret|token|password|credential|api[_-]?key|private[_-]?key)/i.test(key);
}

/**
 * Developer settings intentionally stays read-only until the backend exposes a
 * typed schema, effective-value source, consumer, and restart contract. The
 * former page mixed environment-only and unverified quick controls with the
 * persisted database, which made a successful save look like a runtime apply.
 */
export function ThinClawConfig() {
    const [settings, setSettings] = useState<SettingEntry[]>([]);
    const [query, setQuery] = useState('');
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<string | null>(null);

    const load = useCallback(async () => {
        setIsLoading(true);
        setError(null);
        try {
            const response = await thinclaw.listSettings();
            setSettings(Array.isArray(response.settings) ? response.settings : []);
        } catch (caught) {
            setSettings([]);
            setError(caught instanceof Error ? caught.message : String(caught));
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        void load();
    }, [load]);

    const filtered = useMemo(() => {
        const normalized = query.trim().toLowerCase();
        if (!normalized) return settings;
        return settings.filter((setting) => setting.key.toLowerCase().includes(normalized));
    }, [query, settings]);

    const copyVisible = async () => {
        try {
            await navigator.clipboard.writeText(JSON.stringify(
                Object.fromEntries(filtered.map((setting) => [
                    setting.key,
                    isSensitiveSetting(setting.key) ? '[redacted by Desktop]' : setting.value,
                ])),
                null,
                2,
            ));
            toast.success('Visible developer settings copied');
        } catch {
            toast.error('Unable to copy developer settings');
        }
    };

    if (isLoading) return <AsyncState kind="loading" title="Loading developer settings" className="flex-1" />;

    return (
        <div className="mx-auto flex w-full max-w-6xl flex-1 flex-col gap-5">
            <Notice tone="warning" title="Read-only by design">
                This view only shows stored Agent settings. It does not claim that a value has a live runtime consumer, an apply behavior, or a restart contract. Sensitive-looking values are redacted before display and copy. Provider, model, and Desktop settings belong in global Settings; channel setup belongs in Channels.
            </Notice>

            <Surface className="p-4">
                <div className="flex flex-wrap items-center gap-3">
                    <div className="flex min-w-56 flex-1 items-center gap-2 rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle px-3">
                        <Search className="size-3.5 text-content-muted" aria-hidden="true" />
                        <input
                            aria-label="Filter developer settings"
                            value={query}
                            onChange={(event) => setQuery(event.currentTarget.value)}
                            placeholder="Filter stored settings"
                            className="h-[var(--control-height-compact)] min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-content-muted"
                        />
                    </div>
                    <Button size="sm" variant="secondary" onClick={copyVisible} disabled={filtered.length === 0}>
                        <Copy className="size-3.5" aria-hidden="true" /> Copy visible
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => void load()}>
                        <RefreshCw className="size-3.5" aria-hidden="true" /> Refresh
                    </Button>
                </div>
            </Surface>

            {error ? (
                <AsyncState kind="error" title="Developer settings could not be loaded" description={error} actionLabel="Retry" onAction={() => void load()} />
            ) : filtered.length === 0 ? (
                <AsyncState kind="empty" title={query ? 'No stored settings match this filter' : 'No stored developer settings'} description="A typed, user-facing setting is shown in its owning workflow instead." />
            ) : (
                <div className="space-y-2">
                    {filtered.map((setting) => (
                        <Surface key={setting.key} className="p-4">
                            <div className="flex flex-wrap items-start justify-between gap-2">
                                <div className="min-w-0">
                                    <p className="break-all font-mono text-xs font-semibold text-content-primary">{setting.key}</p>
                                    <p className="mt-1 text-[10px] text-content-muted">
                                        Stored {Number.isNaN(Date.parse(setting.updated_at)) ? setting.updated_at : new Date(setting.updated_at).toLocaleString()}
                                    </p>
                                </div>
                                <Settings2 className="size-4 shrink-0 text-content-muted" aria-hidden="true" />
                            </div>
                            <pre className="mt-3 max-h-48 overflow-auto rounded-[var(--radius-control)] bg-surface-subtle p-3 text-[11px] leading-relaxed text-content-muted">
                                {isSensitiveSetting(setting.key)
                                    ? '[redacted by Desktop]'
                                    : typeof setting.value === 'string' ? setting.value : JSON.stringify(setting.value, null, 2)}
                            </pre>
                        </Surface>
                    ))}
                </div>
            )}
        </div>
    );
}
