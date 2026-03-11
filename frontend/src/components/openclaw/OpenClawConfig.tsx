import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Settings2, RefreshCw, Plus, Edit3, Check, X,
    Search, Copy, Shield, Shrink, GitBranch, Loader2,
    Brain, User, Heart, BookOpen, ExternalLink
} from 'lucide-react';
import * as openclawApi from '../../lib/openclaw';
import { toast } from 'sonner';
import { useChatLayout } from '../chat/ChatProvider';

interface SettingEntry {
    key: string;
    value: any;
    updated_at: string;
    editing: boolean;
    editValue: string;
}

export function OpenClawConfig() {
    const { setActiveOpenClawPage } = useChatLayout();
    const [settings, setSettings] = useState<SettingEntry[]>([]);
    const [loading, setLoading] = useState(true);
    const [search, setSearch] = useState('');
    const [newKey, setNewKey] = useState('');
    const [newValue, setNewValue] = useState('');
    const [showAddForm, setShowAddForm] = useState(false);

    // URL Allowlist state
    const [urlAllowlist, setUrlAllowlist] = useState('');
    const [urlDirty, setUrlDirty] = useState(false);
    const [urlSaving, setUrlSaving] = useState(false);

    // Compaction state
    const [compacting, setCompacting] = useState(false);
    const [compactionResult, setCompactionResult] = useState<openclawApi.CompactSessionResponse | null>(null);

    // Failover state
    const [fallbackModel, setFallbackModel] = useState('');
    const [fallbackDirty, setFallbackDirty] = useState(false);
    const [fallbackSaving, setFallbackSaving] = useState(false);

    const loadSettings = useCallback(async () => {
        setLoading(true);
        try {
            const resp = await openclawApi.listSettings();
            const entries = (resp.settings || []).map(s => ({
                ...s,
                editing: false,
                editValue: typeof s.value === 'string' ? s.value : JSON.stringify(s.value, null, 2),
            }));
            setSettings(entries);

            // Extract URL allowlist
            const urlSetting = entries.find(s => s.key === 'HTTP_URL_ALLOWLIST');
            if (urlSetting) {
                const val = typeof urlSetting.value === 'string' ? urlSetting.value : JSON.stringify(urlSetting.value);
                setUrlAllowlist(val);
            }

            // Extract fallback model
            const fbSetting = entries.find(s => s.key === 'LLM_FALLBACK_MODEL');
            if (fbSetting) {
                setFallbackModel(typeof fbSetting.value === 'string' ? fbSetting.value : '');
            }
        } catch (e) {
            console.error('Failed to list settings', e);
            setSettings([]);
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => { loadSettings(); }, [loadSettings]);

    const handleSave = async (key: string, rawValue: string) => {
        try {
            let parsed: any;
            try { parsed = JSON.parse(rawValue); } catch { parsed = rawValue; }
            await openclawApi.setSetting(key, parsed);
            toast.success(`Saved "${key}"`);
            setSettings(prev => prev.map(s =>
                s.key === key ? { ...s, editing: false, value: parsed, editValue: rawValue } : s
            ));
        } catch (e) {
            toast.error(`Failed to save "${key}"`, { description: String(e) });
        }
    };

    const handleAdd = async () => {
        if (!newKey.trim()) return;
        try {
            let parsed: any;
            try { parsed = JSON.parse(newValue); } catch { parsed = newValue; }
            await openclawApi.setSetting(newKey.trim(), parsed);
            toast.success(`Added "${newKey.trim()}"`);
            setNewKey('');
            setNewValue('');
            setShowAddForm(false);
            loadSettings();
        } catch (e) {
            toast.error('Failed to add setting', { description: String(e) });
        }
    };

    const handleExport = () => {
        const data: Record<string, any> = {};
        settings.forEach(s => { data[s.key] = s.value; });
        navigator.clipboard.writeText(JSON.stringify(data, null, 2));
        toast.success('Settings copied to clipboard');
    };

    const handleSaveUrlAllowlist = async () => {
        setUrlSaving(true);
        try {
            await openclawApi.setSetting('HTTP_URL_ALLOWLIST', urlAllowlist.trim());
            toast.success('URL allowlist saved');
            setUrlDirty(false);
        } catch (e) {
            toast.error('Failed to save URL allowlist', { description: String(e) });
        } finally {
            setUrlSaving(false);
        }
    };

    const handleCompact = async () => {
        setCompacting(true);
        try {
            const result = await openclawApi.compactSession('agent:main');
            setCompactionResult(result);
            toast.success('Compaction analysis complete');
        } catch (e) {
            toast.error('Compaction failed', { description: String(e) });
        } finally {
            setCompacting(false);
        }
    };

    const handleSaveFallback = async () => {
        setFallbackSaving(true);
        try {
            await openclawApi.setSetting('LLM_FALLBACK_MODEL', fallbackModel.trim());
            toast.success('Fallback model saved');
            setFallbackDirty(false);
        } catch (e) {
            toast.error('Failed to save fallback model', { description: String(e) });
        } finally {
            setFallbackSaving(false);
        }
    };

    const filtered = settings.filter(s =>
        s.key.toLowerCase().includes(search.toLowerCase())
    );

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Header */}
            <div className="flex-shrink-0 px-5 pt-5 pb-3">
                <div className="flex items-center justify-between mb-4">
                    <div className="flex items-center gap-3">
                        <div className="w-9 h-9 rounded-xl bg-gradient-to-br from-amber-500/20 to-orange-500/20 border border-amber-500/30 flex items-center justify-center">
                            <Settings2 className="w-4.5 h-4.5 text-amber-400" />
                        </div>
                        <div>
                            <h2 className="text-base font-semibold text-zinc-100">Config Editor</h2>
                            <p className="text-xs text-zinc-500">{settings.length} settings</p>
                        </div>
                    </div>
                    <div className="flex items-center gap-2">
                        <button
                            onClick={handleExport}
                            className="p-2 rounded-lg bg-white/5 border border-border/40 text-zinc-400 hover:text-white hover:bg-white/10 transition-all"
                            title="Export to clipboard"
                        >
                            <Copy className="w-3.5 h-3.5" />
                        </button>
                        <button
                            onClick={() => setShowAddForm(!showAddForm)}
                            className="p-2 rounded-lg bg-amber-500/10 border border-amber-500/30 text-amber-400 hover:bg-amber-500/20 transition-all"
                        >
                            <Plus className="w-3.5 h-3.5" />
                        </button>
                        <button
                            onClick={loadSettings}
                            className="p-2 rounded-lg bg-white/5 border border-border/40 text-zinc-400 hover:text-white hover:bg-white/10 transition-all"
                        >
                            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
                        </button>
                    </div>
                </div>

                {/* Search */}
                <div className="relative">
                    <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-zinc-500" />
                    <input
                        type="text"
                        value={search}
                        onChange={e => setSearch(e.target.value)}
                        placeholder="Search settings..."
                        className="w-full pl-9 pr-3 py-2 rounded-lg bg-white/5 border border-border/40 text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-amber-500/50"
                    />
                </div>
            </div>

            {/* Add Form */}
            <AnimatePresence>
                {showAddForm && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden px-5"
                    >
                        <div className="p-3 rounded-lg bg-amber-500/5 border border-amber-500/20 mb-3 space-y-2">
                            <input
                                type="text"
                                value={newKey}
                                onChange={e => setNewKey(e.target.value)}
                                placeholder="Key name"
                                className="w-full px-3 py-1.5 rounded bg-black/30 border border-border/40 text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-amber-500/50"
                            />
                            <textarea
                                value={newValue}
                                onChange={e => setNewValue(e.target.value)}
                                placeholder="Value (JSON or string)"
                                rows={2}
                                className="w-full px-3 py-1.5 rounded bg-black/30 border border-border/40 text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-amber-500/50 font-mono resize-none"
                            />
                            <div className="flex justify-end gap-2">
                                <button onClick={() => setShowAddForm(false)} className="px-3 py-1 text-xs text-zinc-400 hover:text-white transition-colors">
                                    Cancel
                                </button>
                                <button onClick={handleAdd} className="px-3 py-1 rounded bg-amber-500/20 text-amber-300 text-xs hover:bg-amber-500/30 transition-colors">
                                    <Plus className="w-3 h-3 inline mr-1" />Add
                                </button>
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* Agent Identity — Quick Access to workspace files */}
            <div className="px-5 space-y-3 mb-3">
                <div className="p-4 rounded-lg bg-gradient-to-br from-violet-500/5 to-indigo-500/5 border border-violet-500/15">
                    <div className="flex items-center justify-between mb-3">
                        <div className="flex items-center gap-2">
                            <Brain className="w-3.5 h-3.5 text-violet-400" />
                            <span className="text-xs font-semibold text-violet-300 uppercase tracking-wider">Agent Identity</span>
                        </div>
                        <button
                            onClick={() => setActiveOpenClawPage('brain')}
                            className="flex items-center gap-1 text-[10px] text-violet-400 hover:text-violet-300 transition-colors"
                        >
                            Open The Brain
                            <ExternalLink className="w-3 h-3" />
                        </button>
                    </div>
                    <p className="text-[10px] text-zinc-500 mb-3">
                        These files define who the agent is. They're stored in IronClaw's database and auto-seeded on first run.
                    </p>
                    <div className="grid grid-cols-2 gap-2">
                        {[
                            { file: 'SOUL.md', icon: Heart, label: 'Core Values', desc: 'Behavioral principles & boundaries', color: 'rose' },
                            { file: 'IDENTITY.md', icon: User, label: 'Identity', desc: 'Name, personality, emoji', color: 'blue' },
                            { file: 'USER.md', icon: User, label: 'User Context', desc: 'Your name, timezone, preferences', color: 'emerald' },
                            { file: 'AGENTS.md', icon: BookOpen, label: 'Instructions', desc: 'Session startup routine', color: 'amber' },
                        ].map(item => (
                            <button
                                key={item.file}
                                onClick={() => setActiveOpenClawPage('brain')}
                                className={`flex items-start gap-2.5 p-2.5 rounded-lg bg-black/20 border border-white/5 hover:border-${item.color}-500/30 hover:bg-${item.color}-500/5 transition-all text-left group`}
                            >
                                <item.icon className={`w-3.5 h-3.5 text-${item.color}-400 mt-0.5 shrink-0`} />
                                <div className="min-w-0">
                                    <div className="text-[11px] font-semibold text-zinc-200 group-hover:text-white">{item.label}</div>
                                    <div className="text-[9px] text-zinc-500 truncate">{item.file} — {item.desc}</div>
                                </div>
                            </button>
                        ))}
                    </div>
                </div>
            </div>

            {/* Quick Config Sections */}
            <div className="px-5 space-y-3 mb-3">
                {/* URL Allowlist */}
                <div className="p-3 rounded-lg bg-white/[0.02] border border-white/[0.06]">
                    <div className="flex items-center gap-2 mb-2">
                        <Shield className="w-3.5 h-3.5 text-emerald-400" />
                        <span className="text-xs font-semibold text-emerald-300 uppercase tracking-wider">HTTP URL Allowlist</span>
                    </div>
                    <p className="text-[10px] text-zinc-500 mb-2">
                        Comma-separated domains the HTTP tool is allowed to access. Leave empty for no restrictions.
                    </p>
                    <div className="flex gap-2">
                        <input
                            type="text"
                            value={urlAllowlist}
                            onChange={e => { setUrlAllowlist(e.target.value); setUrlDirty(true); }}
                            placeholder="api.example.com, docs.example.com"
                            className="flex-1 px-3 py-1.5 rounded bg-black/30 border border-border/40 text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-emerald-500/50 font-mono"
                        />
                        <button
                            onClick={handleSaveUrlAllowlist}
                            disabled={!urlDirty || urlSaving}
                            className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${urlDirty
                                ? 'bg-emerald-500/20 text-emerald-300 hover:bg-emerald-500/30 border border-emerald-500/30'
                                : 'bg-white/5 text-zinc-600 border border-white/5'
                                }`}
                        >
                            {urlSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                        </button>
                    </div>
                </div>

                {/* Context Compaction */}
                <div className="p-3 rounded-lg bg-white/[0.02] border border-white/[0.06]">
                    <div className="flex items-center gap-2 mb-2">
                        <Shrink className="w-3.5 h-3.5 text-violet-400" />
                        <span className="text-xs font-semibold text-violet-300 uppercase tracking-wider">Context Compaction</span>
                    </div>
                    <p className="text-[10px] text-zinc-500 mb-2">
                        Analyze context window usage and estimate compaction savings.
                    </p>
                    <div className="flex items-center gap-3">
                        <button
                            onClick={handleCompact}
                            disabled={compacting}
                            className="px-4 py-1.5 rounded text-xs font-medium bg-violet-500/15 text-violet-300 border border-violet-500/25 hover:bg-violet-500/25 transition-all flex items-center gap-2"
                        >
                            {compacting ? <Loader2 className="w-3 h-3 animate-spin" /> : <Shrink className="w-3 h-3" />}
                            Analyze Session
                        </button>
                        {compactionResult && (
                            <div className="flex items-center gap-4 text-[10px] font-mono">
                                <span className="text-zinc-400">
                                    {compactionResult.tokens_before.toLocaleString()} → {compactionResult.tokens_after.toLocaleString()} tokens
                                </span>
                                <span className="text-violet-400">
                                    {compactionResult.turns_removed} turns removable
                                </span>
                                {compactionResult.summary && (
                                    <span className="text-zinc-500 truncate max-w-[200px]" title={compactionResult.summary}>
                                        {compactionResult.summary}
                                    </span>
                                )}
                            </div>
                        )}
                    </div>
                </div>

                {/* Multi-Provider Failover */}
                <div className="p-3 rounded-lg bg-white/[0.02] border border-white/[0.06]">
                    <div className="flex items-center gap-2 mb-2">
                        <GitBranch className="w-3.5 h-3.5 text-blue-400" />
                        <span className="text-xs font-semibold text-blue-300 uppercase tracking-wider">Failover Model</span>
                    </div>
                    <p className="text-[10px] text-zinc-500 mb-2">
                        Fallback model/provider used when the primary LLM fails. Leave empty for default behavior.
                    </p>
                    <div className="flex gap-2">
                        <input
                            type="text"
                            value={fallbackModel}
                            onChange={e => { setFallbackModel(e.target.value); setFallbackDirty(true); }}
                            placeholder="e.g. gpt-4o, claude-3-haiku-20240307"
                            className="flex-1 px-3 py-1.5 rounded bg-black/30 border border-border/40 text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-blue-500/50 font-mono"
                        />
                        <button
                            onClick={handleSaveFallback}
                            disabled={!fallbackDirty || fallbackSaving}
                            className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${fallbackDirty
                                ? 'bg-blue-500/20 text-blue-300 hover:bg-blue-500/30 border border-blue-500/30'
                                : 'bg-white/5 text-zinc-600 border border-white/5'
                                }`}
                        >
                            {fallbackSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                        </button>
                    </div>
                </div>
            </div>

            {/* Settings List */}
            <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-2 mt-2">
                {loading ? (
                    <div className="flex items-center justify-center py-16 text-zinc-500">
                        <RefreshCw className="w-5 h-5 animate-spin mr-2" />
                        Loading...
                    </div>
                ) : filtered.length === 0 ? (
                    <div className="flex flex-col items-center justify-center py-16 text-zinc-500">
                        <Settings2 className="w-8 h-8 mb-3 opacity-30" />
                        <p className="text-sm">{search ? 'No matching settings' : 'No settings stored'}</p>
                    </div>
                ) : (
                    filtered.map((setting, i) => (
                        <motion.div
                            key={setting.key}
                            initial={{ opacity: 0, y: 10 }}
                            animate={{ opacity: 1, y: 0 }}
                            transition={{ delay: i * 0.03 }}
                            className="p-3 rounded-lg bg-white/[0.03] border border-white/[0.06] hover:border-border/40 transition-all group"
                        >
                            <div className="flex items-start justify-between gap-3">
                                <div className="flex-1 min-w-0">
                                    <div className="flex items-center gap-2 mb-1">
                                        <span className="text-sm font-mono font-medium text-amber-300/90">{setting.key}</span>
                                        <span className="text-[10px] text-zinc-600">{new Date(setting.updated_at).toLocaleString()}</span>
                                    </div>
                                    {setting.editing ? (
                                        <div className="flex items-end gap-2 mt-2">
                                            <textarea
                                                value={setting.editValue}
                                                onChange={e => setSettings(prev => prev.map(s =>
                                                    s.key === setting.key ? { ...s, editValue: e.target.value } : s
                                                ))}
                                                rows={3}
                                                className="flex-1 px-3 py-1.5 rounded bg-black/40 border border-amber-500/30 text-sm text-zinc-200 font-mono resize-none focus:outline-none focus:border-amber-500/50"
                                                autoFocus
                                            />
                                            <div className="flex flex-col gap-1">
                                                <button
                                                    onClick={() => handleSave(setting.key, setting.editValue)}
                                                    className="p-1.5 rounded bg-emerald-500/20 text-emerald-400 hover:bg-emerald-500/30 transition-colors"
                                                >
                                                    <Check className="w-3 h-3" />
                                                </button>
                                                <button
                                                    onClick={() => setSettings(prev => prev.map(s =>
                                                        s.key === setting.key ? { ...s, editing: false } : s
                                                    ))}
                                                    className="p-1.5 rounded bg-white/5 text-zinc-400 hover:bg-white/10 transition-colors"
                                                >
                                                    <X className="w-3 h-3" />
                                                </button>
                                            </div>
                                        </div>
                                    ) : (
                                        <pre className="text-xs text-zinc-400 font-mono overflow-x-auto max-h-24 overflow-y-auto whitespace-pre-wrap break-all">
                                            {typeof setting.value === 'string' ? setting.value : JSON.stringify(setting.value, null, 2)}
                                        </pre>
                                    )}
                                </div>
                                {!setting.editing && (
                                    <button
                                        onClick={() => setSettings(prev => prev.map(s =>
                                            s.key === setting.key ? { ...s, editing: true } : s
                                        ))}
                                        className="p-1.5 rounded bg-white/5 text-zinc-500 hover:text-white hover:bg-white/10 transition-all opacity-0 group-hover:opacity-100"
                                    >
                                        <Edit3 className="w-3 h-3" />
                                    </button>
                                )}
                            </div>
                        </motion.div>
                    ))
                )}
            </div>
        </div>
    );
}
