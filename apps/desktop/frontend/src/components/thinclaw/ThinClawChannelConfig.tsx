import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import { SlidersHorizontal, RefreshCw, Save, AlertTriangle } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { thinclawCommands } from '../../lib/generated/thinclaw-commands';

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

type FieldValue = string | boolean;

export function ThinClawChannelConfig() {
    const [schemas, setSchemas] = useState<ConfigSchema[]>([]);
    const [values, setValues] = useState<Record<string, Record<string, FieldValue>>>({});
    const [isLoading, setIsLoading] = useState(true);
    const [saving, setSaving] = useState<string | null>(null);
    const [notice, setNotice] = useState<string | null>(null);

    const load = useCallback(async () => {
        setIsLoading(true);
        setNotice(null);
        try {
            const r = await thinclawCommands.thinclawChannelConfigSchemas();
            if (r.status === 'ok') {
                const data = r.data as { available?: boolean; reason?: string; schemas?: ConfigSchema[] };
                if (data?.available === false) {
                    setNotice(data.reason ?? 'Channel configuration is unavailable in this mode.');
                    setSchemas([]);
                } else {
                    setSchemas(Array.isArray(data?.schemas) ? data.schemas : []);
                }
            } else {
                setNotice(String(r.error));
            }
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        load();
    }, [load]);

    const setField = (channel: string, field: string, val: FieldValue) =>
        setValues((v) => ({ ...v, [channel]: { ...(v[channel] ?? {}), [field]: val } }));

    const fieldValue = (schema: ConfigSchema, field: ConfigField): FieldValue => {
        const current = values[schema.channel_id]?.[field.id];
        if (current !== undefined) return current;
        if (field.field_type === 'checkbox') return field.default_value === true;
        return typeof field.default_value === 'string' ? field.default_value : '';
    };

    const submit = async (schema: ConfigSchema) => {
        setSaving(schema.channel_id);
        const payload = schema.fields.reduce<Record<string, FieldValue>>((acc, f) => {
            acc[f.id] = fieldValue(schema, f);
            return acc;
        }, {});
        const tId = toast.loading(`Saving ${schema.channel_name} configuration…`);
        try {
            const r = await thinclawCommands.thinclawChannelConfigSubmit(schema.channel_id, payload);
            if (r.status === 'ok') {
                const data = r.data as { note?: string };
                toast.success(data?.note ?? 'Configuration saved', { id: tId });
            } else {
                const e = r.error as { reason?: string; message?: string };
                toast.error(e?.reason ?? e?.message ?? String(r.error), { id: tId });
            }
        } finally {
            setSaving(null);
        }
    };

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center">
                <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
            </div>
        );
    }

    return (
        <motion.div className="flex-1 overflow-y-auto p-8 space-y-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-cyan-500/10 border border-cyan-500/20">
                        <SlidersHorizontal className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Channel Configuration</h1>
                        <p className="text-xs text-muted-foreground">
                            Runtime settings for channels that expose a config schema
                        </p>
                    </div>
                </div>
                <button
                    onClick={load}
                    className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                >
                    <RefreshCw className="w-3.5 h-3.5" />
                </button>
            </div>

            {notice && (
                <div className="rounded-xl border border-amber-500/20 bg-amber-500/5 px-4 py-3 flex items-start gap-2">
                    <AlertTriangle className="w-4 h-4 text-amber-400 mt-0.5 shrink-0" />
                    <p className="text-xs text-amber-200/90">{notice}</p>
                </div>
            )}

            {!notice && schemas.length === 0 && (
                <p className="text-xs text-muted-foreground">No channels expose a configuration schema yet.</p>
            )}

            {schemas.map((schema) => (
                <div key={schema.channel_id} className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-6 space-y-4">
                    <div>
                        <h2 className="text-sm font-bold">{schema.channel_name}</h2>
                        {schema.help && <p className="text-[11px] text-muted-foreground mt-0.5">{schema.help}</p>}
                    </div>

                    <div className="space-y-3">
                        {schema.fields.map((field) => {
                            const val = fieldValue(schema, field);
                            return (
                                <div key={field.id} className="space-y-1">
                                    <label className="text-xs font-medium text-foreground/90 flex items-center gap-1">
                                        {field.label}
                                        {field.required && <span className="text-red-400">*</span>}
                                    </label>
                                    {field.field_type === 'checkbox' ? (
                                        <label className="flex items-center gap-2 cursor-pointer">
                                            <input
                                                type="checkbox"
                                                checked={val === true}
                                                onChange={(e) => setField(schema.channel_id, field.id, e.target.checked)}
                                                className="accent-primary"
                                            />
                                            <span className="text-[11px] text-muted-foreground">{field.help_text}</span>
                                        </label>
                                    ) : field.field_type === 'textarea' ? (
                                        <textarea
                                            value={String(val)}
                                            onChange={(e) => setField(schema.channel_id, field.id, e.target.value)}
                                            rows={3}
                                            placeholder={field.help_text ?? ''}
                                            className="w-full rounded-lg border border-border/40 bg-black/20 px-3 py-2 text-xs outline-none focus:border-primary/40"
                                        />
                                    ) : field.field_type === 'select' ? (
                                        <select
                                            value={String(val)}
                                            onChange={(e) => setField(schema.channel_id, field.id, e.target.value)}
                                            className="w-full rounded-lg border border-border/40 bg-black/20 px-3 py-2 text-xs outline-none focus:border-primary/40"
                                        >
                                            {(field.options ?? []).map((o) => (
                                                <option key={o.value} value={o.value}>{o.label}</option>
                                            ))}
                                        </select>
                                    ) : (
                                        <input
                                            type={field.field_type === 'password' ? 'password' : field.field_type === 'number' ? 'number' : 'text'}
                                            value={String(val)}
                                            onChange={(e) => setField(schema.channel_id, field.id, e.target.value)}
                                            placeholder={field.help_text ?? ''}
                                            className="w-full rounded-lg border border-border/40 bg-black/20 px-3 py-2 text-xs outline-none focus:border-primary/40"
                                        />
                                    )}
                                    {field.field_type !== 'checkbox' && field.help_text && (
                                        <p className="text-[10px] text-muted-foreground">{field.help_text}</p>
                                    )}
                                </div>
                            );
                        })}
                    </div>

                    <button
                        onClick={() => submit(schema)}
                        disabled={saving === schema.channel_id}
                        className={cn(
                            'inline-flex items-center gap-1.5 px-4 py-2 rounded-xl text-xs font-medium border transition-all',
                            'bg-primary/15 text-primary border-primary/20 hover:bg-primary/20 disabled:opacity-50',
                        )}
                    >
                        {saving === schema.channel_id ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Save className="w-3.5 h-3.5" />}
                        Save
                    </button>
                </div>
            ))}
        </motion.div>
    );
}
