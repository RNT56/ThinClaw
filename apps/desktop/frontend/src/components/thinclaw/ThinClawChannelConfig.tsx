import { useState, useEffect, useCallback } from 'react';
import { RefreshCw, Save } from 'lucide-react';
import { toast } from 'sonner';
import { thinclawCommands } from '../../lib/generated/thinclaw-commands';
import { AsyncState, Button, Notice, Surface } from '../ui';
import { normalizeAgentActionOutcome } from './action-outcome';

interface ConfigOption {
    value: string;
    label: string;
}
interface ConfigField {
    id: string;
    label: string;
    field_type: string;
    required: boolean;
    help_text?: string | null;
    default_value?: unknown;
    options?: ConfigOption[] | null;
}
interface ConfigSchema {
    channel_id: string;
    channel_name: string;
    fields: ConfigField[];
    help?: string | null;
}

type FieldValue = string | boolean | number | null;
type ChannelValues = Record<string, Record<string, FieldValue>>;

interface ChannelConfigSchemasResponse {
    available?: boolean;
    reason?: string;
    schemas?: ConfigSchema[];
    values?: ChannelValues;
    secret_binding_available?: boolean;
    secret_binding_reason?: string;
}

interface ChannelConfigSubmitResponse {
    ok?: boolean;
    persisted?: boolean;
    forwarded?: boolean;
    restart_required?: boolean;
    note?: string;
}

export function ThinClawChannelConfig() {
    const [schemas, setSchemas] = useState<ConfigSchema[]>([]);
    const [values, setValues] = useState<Record<string, Record<string, FieldValue>>>({});
    const [isLoading, setIsLoading] = useState(true);
    const [saving, setSaving] = useState<string | null>(null);
    const [notice, setNotice] = useState<string | null>(null);
    const [secretBindingAvailable, setSecretBindingAvailable] = useState(false);
    const [secretBindingReason, setSecretBindingReason] = useState<string | null>(null);

    const load = useCallback(async () => {
        setIsLoading(true);
        setNotice(null);
        try {
            const r = await thinclawCommands.thinclawChannelConfigSchemas();
            if (r.status === 'ok') {
                const data = r.data as ChannelConfigSchemasResponse;
                if (data?.available === false) {
                    setNotice(data.reason ?? 'Channel configuration is unavailable in this mode.');
                    setSchemas([]);
                    setValues({});
                } else {
                    setSchemas(Array.isArray(data?.schemas) ? data.schemas : []);
                    setValues(data?.values ?? {});
                    setSecretBindingAvailable(data?.secret_binding_available === true);
                    setSecretBindingReason(data?.secret_binding_reason ?? null);
                }
            } else {
                setNotice(String(r.error));
            }
        } catch (caught) {
            setSchemas([]);
            setValues({});
            setNotice(caught instanceof Error ? caught.message : String(caught));
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        void load();
    }, [load]);

    const setField = (channel: string, field: string, val: FieldValue) =>
        setValues((v) => ({ ...v, [channel]: { ...(v[channel] ?? {}), [field]: val } }));

    const fieldValue = (schema: ConfigSchema, field: ConfigField): FieldValue => {
        const current = values[schema.channel_id]?.[field.id];
        if (current !== undefined) return current;
        if (field.field_type === 'checkbox') return field.default_value === true;
        if (field.field_type === 'number') {
            return typeof field.default_value === 'number' ? field.default_value : null;
        }
        return typeof field.default_value === 'string' ? field.default_value : '';
    };

    const submit = async (schema: ConfigSchema) => {
        const editableFields = schema.fields.filter((field) => field.field_type !== 'password');
        if (editableFields.length === 0) return;
        setSaving(schema.channel_id);
        const payload = editableFields.reduce<Record<string, FieldValue>>((acc, f) => {
            acc[f.id] = fieldValue(schema, f);
            return acc;
        }, {});
        const tId = toast.loading(`Saving ${schema.channel_name} configuration…`);
        try {
            const r = await thinclawCommands.thinclawChannelConfigSubmit(schema.channel_id, payload);
            if (r.status === 'ok') {
                const data = r.data as ChannelConfigSubmitResponse;
                const outcome = normalizeAgentActionOutcome(data, 'Configuration saved');
                const detail = outcome.message;
                if (outcome.state === 'rejected') {
                    setNotice(detail);
                    toast.error(detail, { id: tId });
                } else if (outcome.state === 'persisted' || outcome.state === 'prepared') {
                    setNotice(detail);
                    toast.info(detail, { id: tId });
                } else if (outcome.state === 'restart-required') {
                    setNotice(detail);
                    toast.info(detail, { id: tId });
                } else if (outcome.state === 'unknown') {
                    setNotice(`The request completed, but its apply state is not known. ${detail}`);
                    toast.info(`The request completed, but its apply state is not known. ${detail}`, { id: tId });
                } else {
                    toast.success(detail, { id: tId });
                }
            } else {
                const e = r.error as { reason?: string; message?: string };
                const message = e?.reason ?? e?.message ?? String(r.error);
                setNotice(message);
                toast.error(message, { id: tId });
            }
        } catch (caught) {
            const message = caught instanceof Error ? caught.message : String(caught);
            setNotice(message);
            toast.error(message, { id: tId });
        } finally {
            setSaving(null);
        }
    };

    if (isLoading) {
        return <AsyncState kind="loading" title="Loading channel setup" className="flex-1" />;
    }

    return (
        <section aria-label="Channel setup" className="mx-auto flex w-full max-w-5xl flex-1 flex-col gap-5">
            <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                    <h2 className="text-sm font-semibold">Published channel setup</h2>
                    <p className="mt-1 text-xs text-content-muted">Only fields returned by the selected runtime can be edited here. Stored secret values are never sent back to Desktop.</p>
                </div>
                <Button size="sm" variant="secondary" onClick={() => void load()} aria-label="Refresh channel configuration">
                    <RefreshCw className="size-3.5" aria-hidden="true" /> Refresh
                </Button>
            </div>

            {notice && <Notice tone="warning" title="Channel setup needs attention">{notice}</Notice>}

            {schemas.length === 0 ? (
                <AsyncState kind="empty" title="No channels expose a configuration schema" description="A channel appears here only when the selected runtime publishes a supported setup schema." />
            ) : schemas.map((schema) => (
                <Surface key={schema.channel_id} className="space-y-4 p-5">
                    <div>
                        <h2 className="text-sm font-semibold">{schema.channel_name}</h2>
                        {schema.help && <p className="mt-1 text-xs leading-relaxed text-content-muted">{schema.help}</p>}
                    </div>

                    {schema.fields.length > 0 && <div className="space-y-4">
                        {schema.fields.map((field) => {
                            const val = fieldValue(schema, field);
                            const inputId = `channel-config-${schema.channel_id}-${field.id}`;
                            if (field.field_type === 'password') {
                                return (
                                    <Notice key={field.id} tone="warning" title={`${field.label} is kept outside the renderer`}>
                                        {secretBindingAvailable
                                            ? 'This secret is configured by the supported secure setup flow; Desktop will not display or submit its stored value here.'
                                            : secretBindingReason ?? 'Secret configuration is not available in Desktop yet.'}
                                    </Notice>
                                );
                            }
                            return (
                                <div key={field.id} className="space-y-1.5">
                                    <label htmlFor={inputId} className="flex items-center gap-1 text-xs font-medium text-content-primary">
                                        {field.label}
                                        {field.required && <span aria-label="required" className="text-destructive">*</span>}
                                    </label>
                                    {field.field_type === 'checkbox' ? (
                                        <label className="flex cursor-pointer items-center gap-2 text-xs text-content-muted">
                                            <input id={inputId} type="checkbox" checked={val === true} onChange={(event) => setField(schema.channel_id, field.id, event.target.checked)} className="accent-primary" />
                                            {field.help_text}
                                        </label>
                                    ) : field.field_type === 'textarea' ? (
                                        <textarea id={inputId} value={String(val)} onChange={(event) => setField(schema.channel_id, field.id, event.target.value)} rows={3} placeholder={field.help_text ?? ''} className="w-full rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle px-3 py-2 text-xs text-content-primary outline-none focus-visible:ring-2 focus-visible:ring-primary/20" />
                                    ) : field.field_type === 'select' ? (
                                        <select id={inputId} value={String(val)} onChange={(event) => setField(schema.channel_id, field.id, event.target.value)} className="h-[var(--control-height)] w-full rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle px-3 text-xs text-content-primary outline-none focus-visible:ring-2 focus-visible:ring-primary/20">
                                            {(field.options ?? []).map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                                        </select>
                                    ) : (
                                        <input id={inputId} type={field.field_type === 'number' ? 'number' : 'text'} value={val === null ? '' : String(val)} onChange={(event) => setField(schema.channel_id, field.id, field.field_type === 'number' ? event.target.value === '' ? null : event.target.valueAsNumber : event.target.value)} placeholder={field.help_text ?? ''} className="h-[var(--control-height)] w-full rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle px-3 text-xs text-content-primary outline-none focus-visible:ring-2 focus-visible:ring-primary/20" />
                                    )}
                                    {field.field_type !== 'checkbox' && field.help_text && <p className="text-[10px] text-content-muted">{field.help_text}</p>}
                                </div>
                            );
                        })}
                    </div>}

                    {schema.fields.some((field) => field.field_type !== 'password') && (
                        <Button size="sm" variant="primary" onClick={() => void submit(schema)} disabled={saving === schema.channel_id}>
                            {saving === schema.channel_id ? <RefreshCw className="size-3.5 animate-spin" aria-hidden="true" /> : <Save className="size-3.5" aria-hidden="true" />}
                            Save
                        </Button>
                    )}
                </Surface>
            ))}
        </section>
    );
}
