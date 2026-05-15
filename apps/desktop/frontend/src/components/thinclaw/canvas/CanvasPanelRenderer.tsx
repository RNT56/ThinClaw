/**
 * CanvasPanelRenderer — renders IronClaw UiComponent arrays natively.
 *
 * Handles: Text (markdown-like), Heading, Table, Code, Image, Progress,
 * KeyValue, Divider, Button, Form, and JSON viewer.
 *
 * Button clicks and form submissions dispatch events back to the agent
 * via `canvasDispatchAction()`.
 */

import { useState, useCallback } from 'react';
import { cn } from '../../../lib/utils';
import type {
    UiComponent, ButtonStyle, FormField,
} from '../../../lib/thinclaw';
import { canvasDispatchAction } from '../../../lib/thinclaw';
import {
    ChevronDown, ChevronRight, Copy, Check,
} from 'lucide-react';

interface PanelRendererProps {
    components: UiComponent[];
    sessionKey?: string;
    runId?: string;
}

export function CanvasPanelRenderer({ components, sessionKey, runId }: PanelRendererProps) {
    return (
        <div className="space-y-3 p-4 text-sm">
            {components.map((comp, i) => (
                <ComponentRenderer key={i} component={comp} sessionKey={sessionKey} runId={runId} />
            ))}
        </div>
    );
}

// ── Individual Component Renderers ──────────────────────────────────

function ComponentRenderer({ component: comp, sessionKey, runId }: {
    component: UiComponent; sessionKey?: string; runId?: string;
}) {
    switch (comp.type) {
        case 'text':
            return <TextRenderer content={comp.content} />;
        case 'heading':
            return <HeadingRenderer text={comp.text} level={comp.level ?? 2} />;
        case 'table':
            return <TableRenderer headers={comp.headers} rows={comp.rows} />;
        case 'code':
            return <CodeRenderer language={comp.language} content={comp.content} />;
        case 'image':
            return <ImageRenderer src={comp.src} alt={comp.alt} width={comp.width} />;
        case 'progress':
            return <ProgressRenderer label={comp.label} value={comp.value} max={comp.max} />;
        case 'key_value':
            return <KeyValueRenderer items={comp.items} />;
        case 'divider':
            return <div className="border-t border-white/10 my-2" />;
        case 'button':
            return <ButtonRenderer label={comp.label} action={comp.action} style={comp.style} sessionKey={sessionKey} runId={runId} />;
        case 'form':
            return <FormRenderer formId={comp.form_id} fields={comp.fields} submitLabel={comp.submit_label} sessionKey={sessionKey} runId={runId} />;
        case 'json':
            return <JsonRenderer data={comp.data} collapsed={comp.collapsed} />;
        default:
            return <div className="text-zinc-500 text-xs italic">Unknown component type</div>;
    }
}

// ── Text ────────────────────────────────────────────────────────────

function TextRenderer({ content }: { content: string }) {
    // Simple markdown-like rendering: **bold**, *italic*, `code`, [link](url)
    const parts = content.split(/(\*\*.*?\*\*|\*.*?\*|`[^`]+`|\[.*?\]\(.*?\))/g);
    return (
        <p className="text-zinc-300 leading-relaxed whitespace-pre-wrap">
            {parts.map((part, i) => {
                if (part.startsWith('**') && part.endsWith('**'))
                    return <strong key={i} className="font-bold text-white">{part.slice(2, -2)}</strong>;
                if (part.startsWith('*') && part.endsWith('*'))
                    return <em key={i} className="italic text-zinc-200">{part.slice(1, -1)}</em>;
                if (part.startsWith('`') && part.endsWith('`'))
                    return <code key={i} className="px-1.5 py-0.5 rounded bg-white/5 font-mono text-xs text-cyan-400">{part.slice(1, -1)}</code>;
                const linkMatch = part.match(/^\[(.*?)\]\((.*?)\)$/);
                if (linkMatch)
                    return <a key={i} href={linkMatch[2]} target="_blank" rel="noopener noreferrer" className="text-indigo-400 hover:underline">{linkMatch[1]}</a>;
                return <span key={i}>{part}</span>;
            })}
        </p>
    );
}

// ── Heading ─────────────────────────────────────────────────────────

function HeadingRenderer({ text, level }: { text: string; level: number }) {
    const cls = level === 1
        ? 'text-lg font-bold text-white'
        : level === 2
            ? 'text-base font-semibold text-zinc-100'
            : 'text-sm font-medium text-zinc-200';
    return <div className={cls}>{text}</div>;
}

// ── Table ───────────────────────────────────────────────────────────

function TableRenderer({ headers, rows }: { headers: string[]; rows: string[][] }) {
    return (
        <div className="overflow-x-auto rounded-lg border border-white/10">
            <table className="w-full text-xs">
                <thead>
                    <tr className="border-b border-white/10 bg-white/5">
                        {headers.map((h, i) => (
                            <th key={i} className="px-3 py-2 text-left font-semibold text-zinc-300 uppercase tracking-wider text-[10px]">{h}</th>
                        ))}
                    </tr>
                </thead>
                <tbody>
                    {rows.map((row, ri) => (
                        <tr key={ri} className="border-b border-white/5 hover:bg-white/[0.03] transition-colors">
                            {row.map((cell, ci) => (
                                <td key={ci} className="px-3 py-2 text-zinc-400">{cell}</td>
                            ))}
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}

// ── Code ────────────────────────────────────────────────────────────

function CodeRenderer({ language, content }: { language: string; content: string }) {
    const [copied, setCopied] = useState(false);

    const handleCopy = () => {
        navigator.clipboard.writeText(content);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    return (
        <div className="relative rounded-lg overflow-hidden border border-white/10">
            <div className="flex items-center justify-between px-3 py-1.5 bg-white/5 border-b border-white/10">
                <span className="text-[10px] font-mono text-zinc-500 uppercase">{language}</span>
                <button onClick={handleCopy} className="text-zinc-500 hover:text-white transition-colors p-0.5">
                    {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
                </button>
            </div>
            <pre className="p-3 overflow-x-auto bg-black/30 text-xs font-mono text-zinc-300 leading-relaxed">
                <code>{content}</code>
            </pre>
        </div>
    );
}

// ── Image ───────────────────────────────────────────────────────────

function ImageRenderer({ src, alt, width }: { src: string; alt?: string; width?: number }) {
    return (
        <div className="rounded-lg overflow-hidden border border-white/10">
            <img src={src} alt={alt ?? ''} style={width ? { maxWidth: width } : undefined} className="w-full h-auto" />
            {alt && <p className="text-[10px] text-zinc-500 px-2 py-1 bg-white/5">{alt}</p>}
        </div>
    );
}

// ── Progress ────────────────────────────────────────────────────────

function ProgressRenderer({ label, value, max }: { label?: string; value: number; max: number }) {
    const pct = Math.min(100, Math.round((value / max) * 100));
    return (
        <div>
            {label && <div className="text-xs text-zinc-400 mb-1">{label}</div>}
            <div className="h-2 rounded-full bg-white/5 overflow-hidden">
                <div
                    className="h-full rounded-full bg-gradient-to-r from-indigo-500 to-cyan-500 transition-all duration-500"
                    style={{ width: `${pct}%` }}
                />
            </div>
            <div className="text-[10px] text-zinc-600 mt-0.5 text-right">{pct}%</div>
        </div>
    );
}

// ── Key-Value ───────────────────────────────────────────────────────

function KeyValueRenderer({ items }: { items: { key: string; value: string }[] }) {
    return (
        <div className="space-y-1.5">
            {items.map((item, i) => (
                <div key={i} className="flex items-baseline gap-2">
                    <span className="text-xs text-zinc-500 shrink-0 font-medium">{item.key}</span>
                    <span className="flex-1 border-b border-dotted border-white/10" />
                    <span className="text-xs text-zinc-300 font-mono">{item.value}</span>
                </div>
            ))}
        </div>
    );
}

// ── Button ──────────────────────────────────────────────────────────

function ButtonRenderer({ label, action, style, sessionKey, runId }: {
    label: string; action: string; style?: ButtonStyle; sessionKey?: string; runId?: string;
}) {
    const [loading, setLoading] = useState(false);

    const handleClick = async () => {
        if (!sessionKey) return;
        setLoading(true);
        try {
            await canvasDispatchAction(sessionKey, 'button_click', { action }, runId);
        } catch (e) {
            console.error('[Canvas] Button action failed:', e);
        } finally {
            setLoading(false);
        }
    };

    const styleClasses = {
        primary: 'bg-indigo-500/20 text-indigo-300 border-indigo-500/30 hover:bg-indigo-500/30',
        secondary: 'bg-white/5 text-zinc-300 border-white/10 hover:bg-white/10',
        danger: 'bg-red-500/10 text-red-400 border-red-500/20 hover:bg-red-500/20',
        ghost: 'bg-transparent text-zinc-400 border-transparent hover:bg-white/5 hover:text-zinc-200',
    };

    return (
        <button
            onClick={handleClick}
            disabled={loading || !sessionKey}
            className={cn(
                'inline-flex items-center gap-2 px-4 py-2 rounded-lg text-xs font-semibold border transition-all',
                styleClasses[style ?? 'primary'],
                loading && 'opacity-50 cursor-wait',
                !sessionKey && 'opacity-30 cursor-not-allowed',
            )}
        >
            {loading && <div className="w-3 h-3 border-2 border-current/30 border-t-current rounded-full animate-spin" />}
            {label}
        </button>
    );
}

// ── Form ────────────────────────────────────────────────────────────

function FormRenderer({ formId, fields, submitLabel, sessionKey, runId }: {
    formId: string; fields: FormField[]; submitLabel: string; sessionKey?: string; runId?: string;
}) {
    const [values, setValues] = useState<Record<string, any>>(() => {
        const init: Record<string, any> = {};
        fields.forEach(f => {
            if (f.type === 'checkbox') init[f.name] = f.checked ?? false;
            else init[f.name] = '';
        });
        return init;
    });
    const [submitting, setSubmitting] = useState(false);

    const handleSubmit = useCallback(async (e: React.FormEvent) => {
        e.preventDefault();
        if (!sessionKey) return;
        setSubmitting(true);
        try {
            await canvasDispatchAction(sessionKey, 'form_submit', { form_id: formId, values }, runId);
        } catch (err) {
            console.error('[Canvas] Form submit failed:', err);
        } finally {
            setSubmitting(false);
        }
    }, [sessionKey, formId, values, runId]);

    const inputCls = 'w-full h-9 rounded-lg border border-white/10 bg-white/[0.03] px-3 text-xs font-mono text-zinc-200 focus:ring-1 focus:ring-indigo-500/30 outline-none transition-all';

    return (
        <form onSubmit={handleSubmit} className="space-y-3 p-3 rounded-lg border border-white/10 bg-white/[0.02]">
            {fields.map((field) => (
                <div key={field.name} className="space-y-1">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-zinc-500">
                        {field.label}
                        {field.type === 'text' && field.required && <span className="text-red-400 ml-0.5">*</span>}
                    </label>
                    {field.type === 'text' && (
                        <input
                            type="text"
                            value={values[field.name] ?? ''}
                            onChange={e => setValues(v => ({ ...v, [field.name]: e.target.value }))}
                            placeholder={field.placeholder}
                            required={field.required}
                            className={inputCls}
                        />
                    )}
                    {field.type === 'number' && (
                        <input
                            type="number"
                            value={values[field.name] ?? ''}
                            onChange={e => setValues(v => ({ ...v, [field.name]: e.target.valueAsNumber }))}
                            min={field.min}
                            max={field.max}
                            className={inputCls}
                        />
                    )}
                    {field.type === 'select' && (
                        <select
                            value={values[field.name] ?? ''}
                            onChange={e => setValues(v => ({ ...v, [field.name]: e.target.value }))}
                            className={cn(inputCls, 'appearance-none')}
                        >
                            <option value="">Select…</option>
                            {field.options.map(o => <option key={o} value={o}>{o}</option>)}
                        </select>
                    )}
                    {field.type === 'checkbox' && (
                        <label className="flex items-center gap-2 cursor-pointer">
                            <input
                                type="checkbox"
                                checked={values[field.name] ?? false}
                                onChange={e => setValues(v => ({ ...v, [field.name]: e.target.checked }))}
                                className="rounded border-white/20 bg-white/5 text-indigo-500 focus:ring-indigo-500/30"
                            />
                            <span className="text-xs text-zinc-400">{field.label}</span>
                        </label>
                    )}
                    {field.type === 'textarea' && (
                        <textarea
                            value={values[field.name] ?? ''}
                            onChange={e => setValues(v => ({ ...v, [field.name]: e.target.value }))}
                            rows={field.rows ?? 3}
                            className={cn(inputCls, 'h-auto py-2 resize-y')}
                        />
                    )}
                </div>
            ))}
            <button
                type="submit"
                disabled={submitting || !sessionKey}
                className={cn(
                    'w-full py-2.5 rounded-lg text-xs font-bold uppercase tracking-wider border transition-all',
                    'bg-indigo-500/20 text-indigo-300 border-indigo-500/30 hover:bg-indigo-500/30',
                    submitting && 'opacity-50 cursor-wait',
                )}
            >
                {submitting ? 'Submitting…' : submitLabel}
            </button>
        </form>
    );
}

// ── JSON Viewer ─────────────────────────────────────────────────────

function JsonRenderer({ data, collapsed: initialCollapsed }: { data: any; collapsed?: boolean }) {
    const [collapsed, setCollapsed] = useState(initialCollapsed ?? false);
    const formatted = typeof data === 'string' ? data : JSON.stringify(data, null, 2);

    return (
        <div className="rounded-lg border border-white/10 overflow-hidden">
            <button
                onClick={() => setCollapsed(!collapsed)}
                className="w-full flex items-center gap-1.5 px-3 py-1.5 bg-white/5 text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors"
            >
                {collapsed ? <ChevronRight className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
                <span className="uppercase font-bold tracking-wider">JSON Data</span>
                <span className="text-zinc-600 ml-auto">{typeof data === 'object' ? Object.keys(data).length + ' keys' : ''}</span>
            </button>
            {!collapsed && (
                <pre className="p-3 overflow-x-auto bg-black/30 text-xs font-mono text-emerald-400/80 leading-relaxed max-h-64">
                    <code>{formatted}</code>
                </pre>
            )}
        </div>
    );
}
