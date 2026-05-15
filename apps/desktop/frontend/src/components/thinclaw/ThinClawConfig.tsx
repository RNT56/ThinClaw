import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Settings2, RefreshCw, Plus, Edit3, Check, X,
    Search, Copy, Shield, Shrink, GitBranch, Loader2,
    Brain, User, Heart, BookOpen, ExternalLink, Container, Mail,
    Smartphone, Monitor, Palette, FlaskConical, AlertTriangle, Keyboard
} from 'lucide-react';
import * as thinclawApi from '../../lib/thinclaw';
import { toast } from 'sonner';
import { useChatLayout } from '../chat/ChatProvider';

interface SettingEntry {
    key: string;
    value: any;
    updated_at: string;
    editing: boolean;
    editValue: string;
}

export function ThinClawConfig() {
    const { setActiveThinClawPage } = useChatLayout();
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
    const [compactionResult, setCompactionResult] = useState<thinclawApi.CompactSessionResponse | null>(null);

    // Failover state
    const [fallbackModel, setFallbackModel] = useState('');
    const [fallbackDirty, setFallbackDirty] = useState(false);
    const [fallbackSaving, setFallbackSaving] = useState(false);

    // Claude Code state
    const [ccModel, setCcModel] = useState('');
    const [ccModelDirty, setCcModelDirty] = useState(false);
    const [ccModelSaving, setCcModelSaving] = useState(false);
    const [ccMaxTurns, setCcMaxTurns] = useState('');
    const [ccMaxTurnsDirty, setCcMaxTurnsDirty] = useState(false);
    const [ccMaxTurnsSaving, setCcMaxTurnsSaving] = useState(false);

    // Apple Mail state
    const [amAllowFrom, setAmAllowFrom] = useState('');
    const [amAllowFromDirty, setAmAllowFromDirty] = useState(false);
    const [amPollInterval, setAmPollInterval] = useState('');
    const [amPollDirty, setAmPollDirty] = useState(false);
    const [amSaving, setAmSaving] = useState(false);

    // Codex Code state
    const [codexModel, setCodexModel] = useState('');
    const [codexModelDirty, setCodexModelDirty] = useState(false);
    const [codexModelSaving, setCodexModelSaving] = useState(false);

    // Generic setting save helper (used by Desktop Autonomy, ComfyUI, BlueBubbles, Experiments, Learning)
    const [genericSaving, setGenericSaving] = useState<string | null>(null);

    const loadSettings = useCallback(async () => {
        setLoading(true);
        try {
            const resp = await thinclawApi.listSettings();
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

            // Extract Claude Code settings
            const ccModelSetting = entries.find(s => s.key === 'claude_code_model');
            if (ccModelSetting) {
                setCcModel(typeof ccModelSetting.value === 'string' ? ccModelSetting.value : '');
            }
            const ccTurnsSetting = entries.find(s => s.key === 'claude_code_max_turns');
            if (ccTurnsSetting) {
                setCcMaxTurns(String(ccTurnsSetting.value ?? ''));
            }

            // Extract Apple Mail settings
            const amAllowSetting = entries.find(s => s.key === 'channels.apple_mail_allow_from');
            if (amAllowSetting) {
                setAmAllowFrom(typeof amAllowSetting.value === 'string' ? amAllowSetting.value : '');
            }
            const amPollSetting = entries.find(s => s.key === 'channels.apple_mail_poll_interval');
            if (amPollSetting) {
                setAmPollInterval(String(amPollSetting.value ?? ''));
            }

            // Extract Codex Code settings
            const codexModelSetting = entries.find(s => s.key === 'codex_code_model');
            if (codexModelSetting) {
                setCodexModel(typeof codexModelSetting.value === 'string' ? codexModelSetting.value : '');
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
            await thinclawApi.setSetting(key, parsed);
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
            await thinclawApi.setSetting(newKey.trim(), parsed);
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
            await thinclawApi.setSetting('HTTP_URL_ALLOWLIST', urlAllowlist.trim());
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
            const result = await thinclawApi.compactSession('agent:main');
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
            await thinclawApi.setSetting('LLM_FALLBACK_MODEL', fallbackModel.trim());
            toast.success('Fallback model saved');
            setFallbackDirty(false);
        } catch (e) {
            toast.error('Failed to save fallback model', { description: String(e) });
        } finally {
            setFallbackSaving(false);
        }
    };

    const handleSaveCcModel = async () => {
        setCcModelSaving(true);
        try {
            await thinclawApi.setSetting('claude_code_model', ccModel.trim() || null);
            toast.success('Claude Code model saved');
            setCcModelDirty(false);
        } catch (e) {
            toast.error('Failed to save Claude Code model', { description: String(e) });
        } finally {
            setCcModelSaving(false);
        }
    };

    const handleSaveCcMaxTurns = async () => {
        setCcMaxTurnsSaving(true);
        try {
            const val = ccMaxTurns.trim() ? parseInt(ccMaxTurns.trim(), 10) : null;
            await thinclawApi.setSetting('claude_code_max_turns', val);
            toast.success('Claude Code max turns saved');
            setCcMaxTurnsDirty(false);
        } catch (e) {
            toast.error('Failed to save Claude Code max turns', { description: String(e) });
        } finally {
            setCcMaxTurnsSaving(false);
        }
    };

    const handleSaveAppleMail = async (field: string, value: any) => {
        setAmSaving(true);
        try {
            await thinclawApi.setSetting(field, value);
            toast.success('Apple Mail setting saved');
            if (field === 'channels.apple_mail_allow_from') setAmAllowFromDirty(false);
            if (field === 'channels.apple_mail_poll_interval') setAmPollDirty(false);
        } catch (e) {
            toast.error('Failed to save Apple Mail setting', { description: String(e) });
        } finally {
            setAmSaving(false);
        }
    };

    const handleSaveCodexModel = async () => {
        setCodexModelSaving(true);
        try {
            await thinclawApi.setSetting('codex_code_model', codexModel.trim() || null);
            toast.success('Codex model saved');
            setCodexModelDirty(false);
        } catch (e) {
            toast.error('Failed to save Codex model', { description: String(e) });
        } finally {
            setCodexModelSaving(false);
        }
    };

    /** Generic save for toggle/text settings used by multiple sections */
    const handleGenericSave = async (key: string, value: any, label?: string) => {
        setGenericSaving(key);
        try {
            await thinclawApi.setSetting(key, value);
            toast.success(`${label || key} saved`);
            loadSettings();
        } catch (e) {
            toast.error(`Failed to save ${label || key}`, { description: String(e) });
        } finally {
            setGenericSaving(null);
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
                            <h2 className="text-base font-semibold text-foreground">Config Editor</h2>
                            <p className="text-xs text-muted-foreground">{settings.length} settings</p>
                        </div>
                    </div>
                    <div className="flex items-center gap-2">
                        <button
                            onClick={handleExport}
                            className="p-2 rounded-lg bg-muted/30 border border-border/40 text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-all"
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
                            className="p-2 rounded-lg bg-muted/30 border border-border/40 text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-all"
                        >
                            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
                        </button>
                    </div>
                </div>

                {/* Search */}
                <div className="relative">
                    <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
                    <input
                        type="text"
                        value={search}
                        onChange={e => setSearch(e.target.value)}
                        placeholder="Search settings..."
                        className="w-full pl-9 pr-3 py-2 rounded-lg bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-amber-500/50"
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
                                className="w-full px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-amber-500/50"
                            />
                            <textarea
                                value={newValue}
                                onChange={e => setNewValue(e.target.value)}
                                placeholder="Value (JSON or string)"
                                rows={2}
                                className="w-full px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-amber-500/50 font-mono resize-none"
                            />
                            <div className="flex justify-end gap-2">
                                <button onClick={() => setShowAddForm(false)} className="px-3 py-1 text-xs text-muted-foreground hover:text-foreground transition-colors">
                                    Cancel
                                </button>
                                <button onClick={handleAdd} className="px-3 py-1 rounded bg-amber-500/20 text-amber-600 dark:text-amber-300 text-xs hover:bg-amber-500/30 transition-colors">
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
                            <span className="text-xs font-semibold text-violet-600 dark:text-violet-300 uppercase tracking-wider">Agent Identity</span>
                        </div>
                        <button
                            onClick={() => setActiveThinClawPage('brain')}
                            className="flex items-center gap-1 text-[10px] text-violet-400 hover:text-violet-300 transition-colors"
                        >
                            Open The Brain
                            <ExternalLink className="w-3 h-3" />
                        </button>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-3">
                        These files define who the agent is. They're stored in ThinClaw's database and auto-seeded on first run.
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
                                onClick={() => setActiveThinClawPage('brain')}
                                className={`flex items-start gap-2.5 p-2.5 rounded-lg bg-muted/20 border border-border/30 hover:border-${item.color}-500/30 hover:bg-${item.color}-500/5 transition-all text-left group`}
                            >
                                <item.icon className={`w-3.5 h-3.5 text-${item.color}-400 mt-0.5 shrink-0`} />
                                <div className="min-w-0">
                                    <div className="text-[11px] font-semibold text-foreground group-hover:text-foreground">{item.label}</div>
                                    <div className="text-[9px] text-muted-foreground truncate">{item.file} — {item.desc}</div>
                                </div>
                            </button>
                        ))}
                    </div>
                </div>
            </div>

            {/* Quick Config Sections */}
            <div className="px-5 space-y-3 mb-3">
                {/* URL Allowlist */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <Shield className="w-3.5 h-3.5 text-emerald-400" />
                        <span className="text-xs font-semibold text-emerald-600 dark:text-emerald-300 uppercase tracking-wider">HTTP URL Allowlist</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-2">
                        Comma-separated domains the HTTP tool is allowed to access. Leave empty for no restrictions.
                    </p>
                    <div className="flex gap-2">
                        <input
                            type="text"
                            value={urlAllowlist}
                            onChange={e => { setUrlAllowlist(e.target.value); setUrlDirty(true); }}
                            placeholder="api.example.com, docs.example.com"
                            className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-emerald-500/50 font-mono"
                        />
                        <button
                            onClick={handleSaveUrlAllowlist}
                            disabled={!urlDirty || urlSaving}
                            className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${urlDirty
                                ? 'bg-emerald-500/20 text-emerald-600 dark:text-emerald-300 hover:bg-emerald-500/30 border border-emerald-500/30'
                                : 'bg-muted/30 text-muted-foreground border border-border/30'
                                }`}
                        >
                            {urlSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                        </button>
                    </div>
                </div>

                {/* Context Compaction */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <Shrink className="w-3.5 h-3.5 text-violet-400" />
                        <span className="text-xs font-semibold text-violet-600 dark:text-violet-300 uppercase tracking-wider">Context Compaction</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-2">
                        Analyze context window usage and estimate compaction savings.
                    </p>
                    <div className="flex items-center gap-3">
                        <button
                            onClick={handleCompact}
                            disabled={compacting}
                            className="px-4 py-1.5 rounded text-xs font-medium bg-violet-500/15 text-violet-600 dark:text-violet-300 border border-violet-500/25 hover:bg-violet-500/25 transition-all flex items-center gap-2"
                        >
                            {compacting ? <Loader2 className="w-3 h-3 animate-spin" /> : <Shrink className="w-3 h-3" />}
                            Analyze Session
                        </button>
                        {compactionResult && (
                            <div className="flex items-center gap-4 text-[10px] font-mono">
                                <span className="text-muted-foreground">
                                    {compactionResult.tokens_before.toLocaleString()} → {compactionResult.tokens_after.toLocaleString()} tokens
                                </span>
                                <span className="text-violet-400">
                                    {compactionResult.turns_removed} turns removable
                                </span>
                                {compactionResult.summary && (
                                    <span className="text-muted-foreground/70 truncate max-w-[200px]" title={compactionResult.summary}>
                                        {compactionResult.summary}
                                    </span>
                                )}
                            </div>
                        )}
                    </div>
                </div>

                {/* Multi-Provider Failover */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <GitBranch className="w-3.5 h-3.5 text-blue-400" />
                        <span className="text-xs font-semibold text-blue-600 dark:text-blue-300 uppercase tracking-wider">Failover Model</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-2">
                        Fallback model/provider used when the primary LLM fails. Leave empty for default behavior.
                    </p>
                    <div className="flex gap-2">
                        <input
                            type="text"
                            value={fallbackModel}
                            onChange={e => { setFallbackModel(e.target.value); setFallbackDirty(true); }}
                            placeholder="e.g. gpt-4o, claude-3-haiku-20240307"
                            className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-blue-500/50 font-mono"
                        />
                        <button
                            onClick={handleSaveFallback}
                            disabled={!fallbackDirty || fallbackSaving}
                            className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${fallbackDirty
                                ? 'bg-blue-500/20 text-blue-600 dark:text-blue-300 hover:bg-blue-500/30 border border-blue-500/30'
                                : 'bg-muted/30 text-muted-foreground border border-border/30'
                                }`}
                        >
                            {fallbackSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                        </button>
                    </div>
                </div>

                {/* Claude Code Sandbox */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <Container className="w-3.5 h-3.5 text-orange-400" />
                        <span className="text-xs font-semibold text-orange-600 dark:text-orange-300 uppercase tracking-wider">Claude Code Sandbox</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-3">
                        Configure the Docker-sandboxed Claude Code agent. Changes take effect on the next container spawn.
                    </p>
                    <div className="space-y-2">
                        <div>
                            <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">Model</label>
                            <div className="flex gap-2 mt-1">
                                <input
                                    type="text"
                                    value={ccModel}
                                    onChange={e => { setCcModel(e.target.value); setCcModelDirty(true); }}
                                    placeholder='sonnet, opus, claude-sonnet-4-20250514'
                                    className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-orange-500/50 font-mono"
                                />
                                <button
                                    onClick={handleSaveCcModel}
                                    disabled={!ccModelDirty || ccModelSaving}
                                    className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${ccModelDirty
                                        ? 'bg-orange-500/20 text-orange-600 dark:text-orange-300 hover:bg-orange-500/30 border border-orange-500/30'
                                        : 'bg-muted/30 text-muted-foreground border border-border/30'
                                        }`}
                                >
                                    {ccModelSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                                </button>
                            </div>
                        </div>
                        <div>
                            <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">Max Turns</label>
                            <div className="flex gap-2 mt-1">
                                <input
                                    type="number"
                                    min={1}
                                    value={ccMaxTurns}
                                    onChange={e => { setCcMaxTurns(e.target.value); setCcMaxTurnsDirty(true); }}
                                    placeholder="10"
                                    className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-orange-500/50 font-mono"
                                />
                                <button
                                    onClick={handleSaveCcMaxTurns}
                                    disabled={!ccMaxTurnsDirty || ccMaxTurnsSaving}
                                    className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${ccMaxTurnsDirty
                                        ? 'bg-orange-500/20 text-orange-600 dark:text-orange-300 hover:bg-orange-500/30 border border-orange-500/30'
                                        : 'bg-muted/30 text-muted-foreground border border-border/30'
                                        }`}
                                >
                                    {ccMaxTurnsSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                                </button>
                            </div>
                        </div>
                    </div>
                </div>

                {/* Apple Mail (macOS) */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <Mail className="w-3.5 h-3.5 text-sky-400" />
                        <span className="text-xs font-semibold text-sky-600 dark:text-sky-300 uppercase tracking-wider">Apple Mail</span>
                        <span className="text-[9px] px-1.5 py-0.5 rounded bg-sky-500/10 text-sky-400 border border-sky-500/20 font-bold uppercase tracking-widest">macOS</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-3">
                        Read and send email via the local Mail.app. Requires Apple Mail to be running.
                    </p>
                    <div className="space-y-2">
                        <div>
                            <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">Allowed Senders</label>
                            <div className="flex gap-2 mt-1">
                                <input
                                    type="text"
                                    value={amAllowFrom}
                                    onChange={e => { setAmAllowFrom(e.target.value); setAmAllowFromDirty(true); }}
                                    placeholder="user@example.com, other@example.com"
                                    className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-sky-500/50 font-mono"
                                />
                                <button
                                    onClick={() => handleSaveAppleMail('channels.apple_mail_allow_from', amAllowFrom.trim())}
                                    disabled={!amAllowFromDirty || amSaving}
                                    className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${amAllowFromDirty
                                        ? 'bg-sky-500/20 text-sky-600 dark:text-sky-300 hover:bg-sky-500/30 border border-sky-500/30'
                                        : 'bg-muted/30 text-muted-foreground border border-border/30'
                                        }`}
                                >
                                    {amSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                                </button>
                            </div>
                            <p className="text-[10px] text-muted-foreground/50 mt-0.5">
                                Comma-separated sender emails. Leave empty to process all senders.
                            </p>
                        </div>
                        <div>
                            <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">Poll Interval (seconds)</label>
                            <div className="flex gap-2 mt-1">
                                <input
                                    type="number"
                                    min={5}
                                    max={120}
                                    value={amPollInterval}
                                    onChange={e => { setAmPollInterval(e.target.value); setAmPollDirty(true); }}
                                    placeholder="10"
                                    className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-sky-500/50 font-mono"
                                />
                                <button
                                    onClick={() => handleSaveAppleMail('channels.apple_mail_poll_interval', amPollInterval.trim() ? parseInt(amPollInterval.trim(), 10) : null)}
                                    disabled={!amPollDirty || amSaving}
                                    className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${amPollDirty
                                        ? 'bg-sky-500/20 text-sky-600 dark:text-sky-300 hover:bg-sky-500/30 border border-sky-500/30'
                                        : 'bg-muted/30 text-muted-foreground border border-border/30'
                                        }`}
                                >
                                    {amSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                                </button>
                            </div>
                        </div>
                    </div>
                </div>

                {/* Codex Code Sandbox */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <Container className="w-3.5 h-3.5 text-green-400" />
                        <span className="text-xs font-semibold text-green-600 dark:text-green-300 uppercase tracking-wider">Codex Code Sandbox</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-3">
                        Docker-sandboxed OpenAI Codex CLI agent. Changes take effect on next container spawn.
                    </p>
                    <div className="space-y-2">
                        <div>
                            <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">Model</label>
                            <div className="flex gap-2 mt-1">
                                <input
                                    type="text"
                                    value={codexModel}
                                    onChange={e => { setCodexModel(e.target.value); setCodexModelDirty(true); }}
                                    placeholder='gpt-5.3-codex'
                                    className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-green-500/50 font-mono"
                                />
                                <button
                                    onClick={handleSaveCodexModel}
                                    disabled={!codexModelDirty || codexModelSaving}
                                    className={`px-3 py-1.5 rounded text-xs font-medium transition-all ${codexModelDirty
                                        ? 'bg-green-500/20 text-green-600 dark:text-green-300 hover:bg-green-500/30 border border-green-500/30'
                                        : 'bg-muted/30 text-muted-foreground border border-border/30'
                                        }`}
                                >
                                    {codexModelSaving ? <Loader2 className="w-3 h-3 animate-spin" /> : <Check className="w-3 h-3" />}
                                </button>
                            </div>
                        </div>
                    </div>
                </div>

                {/* Desktop Autonomy */}
                <div className="p-3 rounded-lg bg-gradient-to-br from-red-500/5 to-rose-500/5 border border-red-500/20">
                    <div className="flex items-center gap-2 mb-2">
                        <Monitor className="w-3.5 h-3.5 text-red-400" />
                        <span className="text-xs font-semibold text-red-600 dark:text-red-300 uppercase tracking-wider">Desktop Autonomy</span>
                        <span className="text-[9px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 border border-red-500/20 font-bold uppercase tracking-widest">macOS</span>
                    </div>
                    <div className="flex items-start gap-2 p-2 rounded bg-red-500/5 border border-red-500/15 mb-3">
                        <AlertTriangle className="w-3.5 h-3.5 text-red-400 mt-0.5 shrink-0" />
                        <p className="text-[10px] text-red-400/80">
                            Enables mouse/keyboard control, AppleScript execution, and screen capture. Use with caution on shared machines.
                        </p>
                    </div>
                    <div className="space-y-2">
                        {[
                            { key: 'desktop_autonomy.enabled', label: 'Enabled', desc: 'Master switch for desktop automation' },
                            { key: 'desktop_autonomy.capture_evidence', label: 'Capture evidence', desc: 'Screenshot before/after each action' },
                            { key: 'desktop_autonomy.pause_on_bootstrap_failure', label: 'Pause on failure', desc: 'Halt if bootstrap workspace check fails' },
                        ].map(item => {
                            const setting = settings.find(s => s.key === item.key);
                            const val = setting?.value === true;
                            return (
                                <div key={item.key} className="flex items-center justify-between py-1">
                                    <div>
                                        <div className="text-[11px] font-medium text-foreground">{item.label}</div>
                                        <div className="text-[9px] text-muted-foreground">{item.desc}</div>
                                    </div>
                                    <button
                                        onClick={() => handleGenericSave(item.key, !val, item.label)}
                                        disabled={genericSaving === item.key}
                                        className={`relative w-9 h-5 rounded-full transition-colors ${val ? 'bg-red-500/60' : 'bg-muted/40 border border-border/40'}`}
                                    >
                                        <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow-sm transition-transform ${val ? 'translate-x-4' : ''}`} />
                                    </button>
                                </div>
                            );
                        })}
                        <div className="pt-1 border-t border-border/20">
                            <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">Kill switch hotkey</label>
                            <div className="flex items-center gap-2 mt-1">
                                <Keyboard className="w-3 h-3 text-muted-foreground/50" />
                                <span className="text-[11px] font-mono text-muted-foreground">
                                    {settings.find(s => s.key === 'desktop_autonomy.kill_switch_hotkey')?.value || 'ctrl+option+command+period'}
                                </span>
                            </div>
                        </div>
                    </div>
                </div>

                {/* BlueBubbles */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <Smartphone className="w-3.5 h-3.5 text-indigo-400" />
                        <span className="text-xs font-semibold text-indigo-600 dark:text-indigo-300 uppercase tracking-wider">BlueBubbles</span>
                        <span className="text-[9px] px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-400 border border-indigo-500/20 font-bold uppercase tracking-widest">iMessage</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-3">
                        Cross-platform iMessage bridge via BlueBubbles server. Requires a Mac running the BlueBubbles server app.
                    </p>
                    <div className="space-y-2">
                        {[
                            { key: 'channels.bluebubbles_enabled', label: 'Enabled', type: 'bool' as const },
                            { key: 'channels.bluebubbles_server_url', label: 'Server URL', type: 'text' as const, placeholder: 'http://192.168.1.50:1234' },
                            { key: 'channels.bluebubbles_allow_from', label: 'Allowed contacts', type: 'text' as const, placeholder: '+1234567890, user@icloud.com' },
                        ].map(item => {
                            const setting = settings.find(s => s.key === item.key);
                            if (item.type === 'bool') {
                                const val = setting?.value === true;
                                return (
                                    <div key={item.key} className="flex items-center justify-between py-1">
                                        <div className="text-[11px] font-medium text-foreground">{item.label}</div>
                                        <button
                                            onClick={() => handleGenericSave(item.key, !val, item.label)}
                                            disabled={genericSaving === item.key}
                                            className={`relative w-9 h-5 rounded-full transition-colors ${val ? 'bg-indigo-500/60' : 'bg-muted/40 border border-border/40'}`}
                                        >
                                            <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow-sm transition-transform ${val ? 'translate-x-4' : ''}`} />
                                        </button>
                                    </div>
                                );
                            }
                            return (
                                <div key={item.key}>
                                    <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">{item.label}</label>
                                    <input
                                        type="text"
                                        defaultValue={typeof setting?.value === 'string' ? setting.value : ''}
                                        placeholder={item.placeholder}
                                        onBlur={e => { if (e.target.value !== (setting?.value ?? '')) handleGenericSave(item.key, e.target.value.trim() || null, item.label); }}
                                        className="w-full mt-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-indigo-500/50 font-mono"
                                    />
                                </div>
                            );
                        })}
                    </div>
                </div>

                {/* ComfyUI */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <Palette className="w-3.5 h-3.5 text-pink-400" />
                        <span className="text-xs font-semibold text-pink-600 dark:text-pink-300 uppercase tracking-wider">ComfyUI</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-3">
                        AI image/video generation via a local ComfyUI server.
                    </p>
                    <div className="space-y-2">
                        {[
                            { key: 'comfyui.server_url', label: 'Server URL', placeholder: 'http://127.0.0.1:8188' },
                            { key: 'comfyui.output_dir', label: 'Output directory', placeholder: '~/.thinclaw/comfyui-output' },
                        ].map(item => {
                            const setting = settings.find(s => s.key === item.key);
                            return (
                                <div key={item.key}>
                                    <label className="text-[10px] text-muted-foreground/60 font-bold uppercase tracking-widest">{item.label}</label>
                                    <input
                                        type="text"
                                        defaultValue={typeof setting?.value === 'string' ? setting.value : ''}
                                        placeholder={item.placeholder}
                                        onBlur={e => { if (e.target.value !== (setting?.value ?? '')) handleGenericSave(item.key, e.target.value.trim() || null, item.label); }}
                                        className="w-full mt-1 px-3 py-1.5 rounded bg-muted/30 border border-border/40 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:border-pink-500/50 font-mono"
                                    />
                                </div>
                            );
                        })}
                    </div>
                </div>

                {/* Experiments & Learning */}
                <div className="p-3 rounded-lg bg-muted/10 border border-border/30">
                    <div className="flex items-center gap-2 mb-2">
                        <FlaskConical className="w-3.5 h-3.5 text-cyan-400" />
                        <span className="text-xs font-semibold text-cyan-600 dark:text-cyan-300 uppercase tracking-wider">Experiments & Learning</span>
                    </div>
                    <p className="text-[10px] text-muted-foreground mb-3">
                        Research campaign system and closed-loop self-improvement subsystems.
                    </p>
                    <div className="space-y-2">
                        {[
                            { key: 'experiments.enabled', label: 'Experiments enabled', desc: 'Research UI + experiment APIs' },
                            { key: 'experiments.allow_remote_runners', label: 'Remote runners', desc: 'Allow SSH, Slurm, Kubernetes runners' },
                            { key: 'learning.enabled', label: 'Learning enabled', desc: 'Closed-loop self-improvement' },
                        ].map(item => {
                            const setting = settings.find(s => s.key === item.key);
                            const val = setting?.value === true;
                            return (
                                <div key={item.key} className="flex items-center justify-between py-1">
                                    <div>
                                        <div className="text-[11px] font-medium text-foreground">{item.label}</div>
                                        <div className="text-[9px] text-muted-foreground">{item.desc}</div>
                                    </div>
                                    <button
                                        onClick={() => handleGenericSave(item.key, !val, item.label)}
                                        disabled={genericSaving === item.key}
                                        className={`relative w-9 h-5 rounded-full transition-colors ${val ? 'bg-cyan-500/60' : 'bg-muted/40 border border-border/40'}`}
                                    >
                                        <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow-sm transition-transform ${val ? 'translate-x-4' : ''}`} />
                                    </button>
                                </div>
                            );
                        })}
                    </div>
                </div>
            </div>

            {/* Settings List */}
            <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-2 mt-2">
                {loading ? (
                    <div className="flex items-center justify-center py-16 text-muted-foreground">
                        <RefreshCw className="w-5 h-5 animate-spin mr-2" />
                        Loading...
                    </div>
                ) : filtered.length === 0 ? (
                    <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
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
                            className="p-3 rounded-lg bg-muted/10 border border-border/30 hover:border-border/50 transition-all group"
                        >
                            <div className="flex items-start justify-between gap-3">
                                <div className="flex-1 min-w-0">
                                    <div className="flex items-center gap-2 mb-1">
                                        <span className="text-sm font-mono font-medium text-amber-600 dark:text-amber-300/90">{setting.key}</span>
                                        <span className="text-[10px] text-muted-foreground/60">{new Date(setting.updated_at).toLocaleString()}</span>
                                    </div>
                                    {setting.editing ? (
                                        <div className="flex items-end gap-2 mt-2">
                                            <textarea
                                                value={setting.editValue}
                                                onChange={e => setSettings(prev => prev.map(s =>
                                                    s.key === setting.key ? { ...s, editValue: e.target.value } : s
                                                ))}
                                                rows={3}
                                                className="flex-1 px-3 py-1.5 rounded bg-muted/30 border border-amber-500/30 text-sm text-foreground font-mono resize-none focus:outline-none focus:border-amber-500/50"
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
                                                    className="p-1.5 rounded bg-muted/30 text-muted-foreground hover:bg-muted/50 transition-colors"
                                                >
                                                    <X className="w-3 h-3" />
                                                </button>
                                            </div>
                                        </div>
                                    ) : (
                                        <pre className="text-xs text-muted-foreground font-mono overflow-x-auto max-h-24 overflow-y-auto whitespace-pre-wrap break-all">
                                            {typeof setting.value === 'string' ? setting.value : JSON.stringify(setting.value, null, 2)}
                                        </pre>
                                    )}
                                </div>
                                {!setting.editing && (
                                    <button
                                        onClick={() => setSettings(prev => prev.map(s =>
                                            s.key === setting.key ? { ...s, editing: true } : s
                                        ))}
                                        className="p-1.5 rounded bg-muted/30 text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-all opacity-0 group-hover:opacity-100"
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
