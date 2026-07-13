import { useMemo, useState } from 'react';
import { BrainCircuit, CircleOff, Database, Loader2, Save, ShieldCheck } from 'lucide-react';
import { toast } from 'sonner';
import * as thinclaw from '../../../lib/thinclaw';
import { cn } from '../../../lib/utils';

type ProviderStatus = {
    provider?: string;
    active?: boolean;
    enabled?: boolean;
    healthy?: boolean;
    readiness?: string;
    latency_ms?: number | null;
    error?: string | null;
};

const PROVIDERS = [
    { id: 'openmemory', label: 'OpenMemory' },
    { id: 'mem0', label: 'Mem0' },
    { id: 'honcho', label: 'Honcho' },
    { id: 'zep', label: 'Zep' },
    { id: 'letta', label: 'Letta' },
    { id: 'chroma', label: 'Chroma' },
    { id: 'qdrant', label: 'Qdrant' },
    { id: 'custom_http', label: 'Custom HTTP' },
] as const;

type FormState = {
    provider: string;
    baseUrl: string;
    apiKeyEnv: string;
    embeddingUrl: string;
    embeddingApiKeyEnv: string;
    collection: string;
    collectionId: string;
    agentId: string;
    providerUserId: string;
    cadence: string;
    depth: string;
    enabled: boolean;
    activate: boolean;
    userModelingEnabled: boolean;
};

const initialForm: FormState = {
    provider: 'openmemory',
    baseUrl: '',
    apiKeyEnv: '',
    embeddingUrl: '',
    embeddingApiKeyEnv: '',
    collection: '',
    collectionId: '',
    agentId: '',
    providerUserId: '',
    cadence: '',
    depth: '',
    enabled: true,
    activate: true,
    userModelingEnabled: false,
};

function optional(value: string): string | null {
    return value.trim() || null;
}

function optionalNumber(value: string): number | null {
    const trimmed = value.trim();
    return trimmed ? Number(trimmed) : null;
}

function defaultEndpoint(provider: string): string {
    switch (provider) {
        case 'openmemory': return 'http://localhost:8888';
        case 'mem0': return 'https://api.mem0.ai';
        case 'letta': return 'https://api.letta.com';
        case 'chroma': return 'http://localhost:8000';
        case 'qdrant': return 'http://localhost:6333';
        default: return 'https://memory.example.com';
    }
}

export function ExternalMemoryPanel({
    providers,
    onChanged,
}: {
    providers: ProviderStatus[];
    onChanged: () => Promise<unknown>;
}) {
    const [form, setForm] = useState<FormState>(initialForm);
    const [busy, setBusy] = useState<'save' | 'disable' | null>(null);
    const selectedStatus = useMemo(
        () => providers.find(provider => provider.provider === form.provider),
        [form.provider, providers],
    );
    const activeStatus = useMemo(
        () => providers.find(provider => provider.active),
        [providers],
    );
    const vectorProvider = form.provider === 'chroma' || form.provider === 'qdrant';

    const update = <Key extends keyof FormState>(key: Key, value: FormState[Key]) => {
        setForm(previous => ({ ...previous, [key]: value }));
    };

    const submit = async () => {
        setBusy('save');
        try {
            await thinclaw.configureExternalMemoryProvider({
                provider: form.provider,
                base_url: optional(form.baseUrl),
                api_key_env: optional(form.apiKeyEnv),
                embedding_url: optional(form.embeddingUrl),
                embedding_api_key_env: optional(form.embeddingApiKeyEnv),
                collection: form.provider === 'qdrant' ? optional(form.collection) : null,
                collection_id: form.provider === 'chroma' ? optional(form.collectionId) : null,
                agent_id: ['mem0', 'letta'].includes(form.provider) ? optional(form.agentId) : null,
                provider_user_id: optional(form.providerUserId),
                enabled: form.enabled,
                activate: form.activate,
                cadence: optionalNumber(form.cadence),
                depth: optionalNumber(form.depth),
                user_modeling_enabled: form.userModelingEnabled,
            });
            toast.success(`${PROVIDERS.find(provider => provider.id === form.provider)?.label} settings saved`);
            await onChanged();
        } catch (error) {
            toast.error(String(error));
        } finally {
            setBusy(null);
        }
    };

    const disable = async () => {
        setBusy('disable');
        try {
            await thinclaw.disableExternalMemoryProvider();
            toast.success('External memory deactivated');
            await onChanged();
        } catch (error) {
            toast.error(String(error));
        } finally {
            setBusy(null);
        }
    };

    return (
        <section className="rounded-xl border border-border/40 bg-card/30 p-5" aria-labelledby="external-memory-title">
            <div className="flex flex-wrap items-start justify-between gap-4">
                <div className="flex items-start gap-3">
                    <div className="rounded-lg border border-primary/20 bg-primary/10 p-2">
                        <Database className="h-4 w-4 text-primary" />
                    </div>
                    <div>
                        <h2 id="external-memory-title" className="text-sm font-bold">External Memory</h2>
                        <p className="mt-0.5 max-w-2xl text-xs text-muted-foreground">
                            Connect long-term memory for recall and turn export. Credentials stay in your environment; ThinClaw stores only the variable name.
                        </p>
                    </div>
                </div>
                <div className="flex items-center gap-2 text-[11px]">
                    <span className={cn(
                        'rounded-full border px-2.5 py-1 font-semibold',
                        activeStatus?.healthy
                            ? 'border-emerald-500/25 bg-emerald-500/10 text-emerald-300'
                            : activeStatus
                                ? 'border-amber-500/25 bg-amber-500/10 text-amber-200'
                                : 'border-border/50 bg-white/3 text-muted-foreground',
                    )}>
                        {activeStatus ? `${activeStatus.provider} · ${activeStatus.readiness || 'unknown'}` : 'No active provider'}
                    </span>
                    <button
                        type="button"
                        onClick={disable}
                        disabled={!activeStatus || busy !== null}
                        className="inline-flex items-center gap-1.5 rounded-lg border border-border/50 px-2.5 py-1 text-muted-foreground transition-colors hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
                    >
                        {busy === 'disable' ? <Loader2 className="h-3 w-3 animate-spin" /> : <CircleOff className="h-3 w-3" />}
                        Deactivate
                    </button>
                </div>
            </div>

            <div className="mt-5 grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1.15fr)_minmax(18rem,0.85fr)]">
                <div className="space-y-4 rounded-xl border border-border/40 bg-background/30 p-4">
                    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                        <label className="space-y-1.5 text-xs font-medium">
                            <span>Provider</span>
                            <select
                                aria-label="External memory provider"
                                value={form.provider}
                                onChange={event => update('provider', event.target.value)}
                                className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50"
                            >
                                {PROVIDERS.map(provider => <option key={provider.id} value={provider.id}>{provider.label}</option>)}
                            </select>
                        </label>
                        <label className="space-y-1.5 text-xs font-medium">
                            <span>Provider endpoint</span>
                            <input
                                aria-label="Provider endpoint"
                                type="url"
                                value={form.baseUrl}
                                onChange={event => update('baseUrl', event.target.value)}
                                placeholder={defaultEndpoint(form.provider)}
                                className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50"
                            />
                        </label>
                        <label className="space-y-1.5 text-xs font-medium">
                            <span>API key environment variable</span>
                            <input
                                aria-label="API key environment variable"
                                value={form.apiKeyEnv}
                                onChange={event => update('apiKeyEnv', event.target.value)}
                                placeholder="THINCLAW_MEMORY_API_KEY"
                                autoCapitalize="none"
                                autoComplete="off"
                                spellCheck={false}
                                className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 font-mono outline-hidden focus:border-primary/50"
                            />
                        </label>
                        <label className="space-y-1.5 text-xs font-medium">
                            <span>Provider user ID <span className="font-normal text-muted-foreground">(optional)</span></span>
                            <input
                                aria-label="Provider user ID"
                                value={form.providerUserId}
                                onChange={event => update('providerUserId', event.target.value)}
                                placeholder="Defaults to local_user"
                                className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50"
                            />
                        </label>
                    </div>

                    {['mem0', 'letta'].includes(form.provider) && (
                        <label className="block space-y-1.5 text-xs font-medium">
                            <span>Provider agent ID {form.provider === 'letta' && <span className="text-amber-300">(required)</span>}</span>
                            <input
                                aria-label="Provider agent ID"
                                value={form.agentId}
                                onChange={event => update('agentId', event.target.value)}
                                className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50"
                            />
                        </label>
                    )}

                    {vectorProvider && (
                        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                            <label className="space-y-1.5 text-xs font-medium sm:col-span-2">
                                <span>Embedding endpoint <span className="text-amber-300">(required)</span></span>
                                <input
                                    aria-label="Embedding endpoint"
                                    type="url"
                                    value={form.embeddingUrl}
                                    onChange={event => update('embeddingUrl', event.target.value)}
                                    placeholder="http://localhost:8080/v1/embeddings"
                                    className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50"
                                />
                            </label>
                            <label className="space-y-1.5 text-xs font-medium">
                                <span>Embedding key environment variable</span>
                                <input
                                    aria-label="Embedding key environment variable"
                                    value={form.embeddingApiKeyEnv}
                                    onChange={event => update('embeddingApiKeyEnv', event.target.value)}
                                    placeholder="EMBEDDING_API_KEY"
                                    autoCapitalize="none"
                                    autoComplete="off"
                                    spellCheck={false}
                                    className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 font-mono outline-hidden focus:border-primary/50"
                                />
                            </label>
                            <label className="space-y-1.5 text-xs font-medium">
                                <span>{form.provider === 'chroma' ? 'Collection ID' : 'Collection'} <span className="text-amber-300">(required)</span></span>
                                <input
                                    aria-label={form.provider === 'chroma' ? 'Collection ID' : 'Collection'}
                                    value={form.provider === 'chroma' ? form.collectionId : form.collection}
                                    onChange={event => update(form.provider === 'chroma' ? 'collectionId' : 'collection', event.target.value)}
                                    className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50"
                                />
                            </label>
                        </div>
                    )}

                    <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                        <label className="space-y-1.5 text-xs font-medium">
                            <span>Cadence</span>
                            <input aria-label="Provider cadence" type="number" min="1" value={form.cadence} onChange={event => update('cadence', event.target.value)} className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50" />
                        </label>
                        <label className="space-y-1.5 text-xs font-medium">
                            <span>Depth</span>
                            <input aria-label="Provider depth" type="number" min="1" value={form.depth} onChange={event => update('depth', event.target.value)} className="w-full rounded-lg border border-border/50 bg-background/70 px-3 py-2 outline-hidden focus:border-primary/50" />
                        </label>
                    </div>

                    <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-xs">
                        <label className="inline-flex items-center gap-2"><input type="checkbox" checked={form.enabled} onChange={event => update('enabled', event.target.checked)} /> Enabled</label>
                        <label className="inline-flex items-center gap-2"><input type="checkbox" checked={form.activate} onChange={event => update('activate', event.target.checked)} /> Make active</label>
                        <label className="inline-flex items-center gap-2"><input type="checkbox" checked={form.userModelingEnabled} onChange={event => update('userModelingEnabled', event.target.checked)} /> User-model prompt context</label>
                    </div>

                    <button
                        type="button"
                        onClick={submit}
                        disabled={busy !== null}
                        className="inline-flex items-center gap-2 rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs font-semibold text-primary transition-colors hover:bg-primary/15 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                        {busy === 'save' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
                        Save provider
                    </button>
                </div>

                <div className="space-y-3">
                    <div className="rounded-xl border border-border/40 bg-background/30 p-4">
                        <div className="flex items-center gap-2 text-xs font-bold">
                            {selectedStatus?.healthy ? <ShieldCheck className="h-4 w-4 text-emerald-300" /> : <BrainCircuit className="h-4 w-4 text-amber-300" />}
                            Selected provider status
                        </div>
                        <div className="mt-3 text-lg font-bold capitalize">{form.provider.replace('_', ' ')}</div>
                        <div className="mt-1 text-xs text-muted-foreground">
                            {selectedStatus
                                ? `${selectedStatus.enabled ? 'Enabled' : 'Disabled'} · ${selectedStatus.active ? 'Active' : 'Inactive'} · ${selectedStatus.readiness || 'Unknown'}`
                                : 'No health record loaded.'}
                        </div>
                        {selectedStatus?.latency_ms != null && <div className="mt-2 text-[11px] text-muted-foreground">Probe latency: {selectedStatus.latency_ms} ms</div>}
                        {selectedStatus?.error && <p role="alert" className="mt-3 rounded-lg border border-amber-500/20 bg-amber-500/10 p-2.5 text-[11px] text-amber-100">{selectedStatus.error}</p>}
                    </div>
                    <div className="rounded-xl border border-border/40 bg-background/30 p-4 text-[11px] leading-relaxed text-muted-foreground">
                        Set the named environment variable before starting Desktop. Hosted Mem0 and Letta need a key; local OpenMemory can run without one. Chroma and Qdrant also need an embedding endpoint and collection.
                    </div>
                </div>
            </div>
        </section>
    );
}
