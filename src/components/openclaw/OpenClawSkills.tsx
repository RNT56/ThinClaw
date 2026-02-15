import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    ToggleLeft,
    ToggleRight,
    RefreshCw,
    Search,
    Shield,
    Zap,
    Package,
    CheckCircle2,
    Github,
    Plus,
    ExternalLink,
    AlertCircle,
    Info,
    AlertTriangle
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';



function SkillCard({ skill, onToggle }: { skill: openclaw.Skill, onToggle: (key: string, enabled: boolean) => void }) {
    const [isToggling, setIsToggling] = useState(false);
    const [isInstalling, setIsInstalling] = useState(false);
    // Force disabled visual if not eligible
    const enabled = !skill.disabled && skill.eligible;

    const handleToggle = async () => {
        setIsToggling(true);
        try {
            await onToggle(skill.skillKey, !enabled);
        } finally {
            setIsToggling(false);
        }
    };

    const handleInstallDeps = async () => {
        if (!skill.install || skill.install.length === 0) return;
        setIsInstalling(true);
        try {
            // We take the first available install option for now
            const option = skill.install[0];
            await openclaw.installOpenClawSkillDeps(skill.skillKey, option.installId);
            toast.success(`Started installation for ${skill.name} dependencies`);
        } catch (e) {
            toast.error(`Failed to install dependencies: ${e}`);
        } finally {
            setIsInstalling(false);
        }
    };

    return (
        <div className={cn(
            "p-5 rounded-2xl border transition-all duration-300 flex flex-col h-full",
            enabled
                ? "bg-primary/[0.03] border-primary/20 shadow-sm shadow-primary/5"
                : "bg-white/[0.02] border-white/5 opacity-80"
        )}>
            <div className="flex items-start justify-between mb-4">
                <div className={cn(
                    "p-2.5 rounded-xl border transition-colors flex items-center justify-center text-xl",
                    enabled ? "bg-primary/10 border-primary/20 text-primary" : "bg-white/5 border-white/10 text-muted-foreground"
                )}>
                    {skill.emoji || (skill.source === 'openclaw-engine-bundled' ? <Package className="w-5 h-5" /> : <Github className="w-5 h-5" />)}
                </div>
                <div className="flex items-center gap-2">
                    {!skill.eligible && skill.install && skill.install.length > 0 && (
                        <button
                            onClick={handleInstallDeps}
                            disabled={isInstalling}
                            className="text-[10px] font-bold uppercase tracking-wider px-2 py-1 rounded bg-amber-500/10 text-amber-500 border border-amber-500/20 hover:bg-amber-500/20 transition-colors flex items-center gap-1"
                        >
                            {isInstalling ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Plus className="w-3 h-3" />}
                            Fix
                        </button>
                    )}
                    <button
                        onClick={handleToggle}
                        disabled={isToggling || (!skill.eligible && !enabled)}
                        className={cn(
                            "transition-all",
                            enabled ? "text-primary hover:opacity-80" : "text-muted-foreground",
                            !skill.eligible && !enabled ? "cursor-not-allowed opacity-30" : "hover:text-foreground"
                        )}
                        title={!skill.eligible && !enabled ? "Fix dependencies before activating" : ""}
                    >
                        {isToggling ? (
                            <RefreshCw className="w-6 h-6 animate-spin" />
                        ) : enabled ? (
                            <ToggleRight className="w-8 h-8" />
                        ) : (
                            <ToggleLeft className="w-8 h-8" />
                        )}
                    </button>
                </div>
            </div>

            <div className="flex-1">
                <div className="flex items-center gap-2">
                    <h3 className="font-semibold">{skill.name}</h3>
                    {enabled && <CheckCircle2 className="w-3.5 h-3.5 text-primary" />}
                </div>
                <p className="text-xs text-muted-foreground mt-1 line-clamp-2 leading-relaxed">
                    {skill.description}
                </p>

                {!skill.eligible && !skill.disabled && (
                    <div className="mt-2 flex items-center gap-1.5 text-[10px] text-amber-500 font-medium">
                        <AlertCircle className="w-3 h-3" />
                        Missing dependencies: {skill.missing?.bins?.join(', ') || 'Unknown'}
                    </div>
                )}
            </div>

            <div className="mt-6 flex items-center justify-between">
                <div className="flex items-center gap-2">
                    <div className="flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-white/5 border border-white/5">
                        <span className="text-[10px] font-bold uppercase tracking-tighter text-muted-foreground/80">
                            {skill.source === 'openclaw-engine-bundled' ? 'Core' : skill.source.replace('openclaw-engine-', '')}
                        </span>
                    </div>
                    {!skill.eligible && (
                        <div className="px-2 py-0.5 rounded-full bg-amber-500/10 border border-amber-500/20">
                            <span className="text-[9px] font-bold uppercase tracking-tight text-amber-500">
                                Requires Setup
                            </span>
                        </div>
                    )}
                </div>
                {skill.homepage && (
                    <a
                        href={skill.homepage}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="p-1 hover:bg-white/5 rounded-md text-muted-foreground transition-colors"
                    >
                        <ExternalLink className="w-3 h-3" />
                    </a>
                )}
            </div>
        </div>
    );
}

export function OpenClawSkills() {
    const [skills, setSkills] = useState<openclaw.Skill[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [search, setSearch] = useState('');
    const [showMarketplace, setShowMarketplace] = useState(false);
    const [repoUrl, setRepoUrl] = useState('');
    const [isInstalling, setIsInstalling] = useState(false);
    const [gatewayMode, setGatewayMode] = useState('local');

    const fetchData = async () => {
        try {
            const [status, data] = await Promise.all([
                openclaw.getOpenClawStatus(),
                openclaw.getOpenClawSkillsStatus()
            ]);
            setGatewayMode(status.gateway_mode);
            setSkills(Array.isArray(data?.skills) ? data.skills : []);
        } catch (e) {
            console.error('Failed to fetch skills:', e);
            toast.error('Failed to sync with Skill Registry');
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchData();
    }, []);

    const handleToggle = async (key: string, enabled: boolean) => {
        try {
            await openclaw.toggleOpenClawSkill(key, enabled);
            setSkills(prev => prev.map(s => s.skillKey === key ? { ...s, disabled: !enabled } : s));
            toast.success(`${enabled ? 'Enabled' : 'Disabled'} ${key}`);
        } catch (e) {
            toast.error(`Failed to toggle skill: ${e}`);
            fetchData();
        }
    };

    const handleInstallRepo = async () => {
        if (!repoUrl) return;
        setIsInstalling(true);
        try {
            const result = await openclaw.installOpenClawSkillRepo(repoUrl);
            toast.success(result);
            setRepoUrl('');
            setShowMarketplace(false);
            fetchData();
        } catch (e) {
            toast.error(`Install failed: ${e}`);
        } finally {
            setIsInstalling(false);
        }
    };

    const filteredSkills = skills.filter(s =>
        s.name.toLowerCase().includes(search.toLowerCase()) ||
        s.skillKey.toLowerCase().includes(search.toLowerCase())
    );

    const activeCount = skills.filter(s => !s.disabled && s.eligible).length;
    const totalCount = skills.length;

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 flex flex-col h-full overflow-hidden"
        >
            <div className="p-8 pb-4 space-y-6 flex-none max-w-6xl w-full mx-auto">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                    <div>
                        <h1 className="text-3xl font-bold tracking-tight">Skill Matrix</h1>
                        <p className="text-muted-foreground mt-1">Manage modular toolsets and agent capabilities.</p>
                    </div>

                    <div className="flex items-center gap-3">
                        <div className="relative">
                            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                            <input
                                type="text"
                                placeholder="Search skills..."
                                value={search}
                                onChange={(e) => setSearch(e.target.value)}
                                className="pl-9 pr-4 py-2 rounded-xl bg-card border border-white/10 focus:border-primary/50 focus:ring-1 focus:ring-primary/50 transition-all text-sm w-64 shadow-inner"
                            />
                        </div>
                        {gatewayMode === 'remote' ? (
                            <div className="flex items-center gap-2 px-4 py-2 rounded-xl border bg-amber-500/10 border-amber-500/20 text-amber-500 text-xs font-medium cursor-help" title="Skill installation is disabled because the local file system is not accessible to the remote gateway.">
                                <AlertTriangle className="w-4 h-4" />
                                Remote: Read-Only
                            </div>
                        ) : (
                            <button
                                onClick={() => setShowMarketplace(!showMarketplace)}
                                className={cn(
                                    "flex items-center gap-2 px-4 py-2 rounded-xl border transition-all text-sm font-bold shadow-sm",
                                    showMarketplace
                                        ? "bg-primary text-primary-foreground border-primary"
                                        : "bg-card border-white/10 hover:bg-white/5"
                                )}
                            >
                                <Plus className="w-4 h-4" />
                                Add Skills
                            </button>
                        )}
                        <div className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary flex items-center gap-2 text-sm font-bold shadow-lg shadow-primary/5">
                            <Zap className="w-4 h-4 fill-current" />
                            {activeCount} / {totalCount} active
                        </div>
                        <button
                            onClick={() => {
                                setIsLoading(true);
                                fetchData();
                            }}
                            className="p-2.5 rounded-xl bg-card border border-white/10 hover:bg-white/5 transition-colors shadow-sm"
                        >
                            <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                        </button>
                    </div>
                </div>

                <AnimatePresence>
                    {showMarketplace && (
                        <motion.div
                            initial={{ opacity: 0, height: 0 }}
                            animate={{ opacity: 1, height: 'auto' }}
                            exit={{ opacity: 0, height: 0 }}
                            className="overflow-hidden"
                        >
                            <div className="p-6 rounded-2xl border bg-card border-white/10 space-y-4 shadow-2xl">
                                <div className="flex items-center gap-3">
                                    <div className="p-2 bg-primary/10 rounded-lg">
                                        <Github className="w-5 h-5 text-primary" />
                                    </div>
                                    <div>
                                        <h3 className="font-semibold text-sm">Install Skill Repository</h3>
                                        <p className="text-xs text-muted-foreground">Clone a collection of tools directly from GitHub into your workspace.</p>
                                    </div>
                                </div>
                                <div className="flex gap-3">
                                    <input
                                        type="text"
                                        placeholder="https://github.com/openclaw/skills"
                                        value={repoUrl}
                                        onChange={(e) => setRepoUrl(e.target.value)}
                                        className="flex-1 px-4 py-2.5 rounded-xl bg-white/5 border border-white/10 focus:border-primary/50 focus:ring-1 focus:ring-primary/50 transition-all text-sm"
                                    />
                                    <button
                                        onClick={handleInstallRepo}
                                        disabled={isInstalling || !repoUrl}
                                        className="px-6 py-2.5 rounded-xl bg-primary text-primary-foreground text-sm font-bold hover:opacity-90 transition-all disabled:opacity-50 flex items-center gap-2"
                                    >
                                        {isInstalling ? (
                                            <RefreshCw className="w-4 h-4 animate-spin" />
                                        ) : (
                                            <Plus className="w-4 h-4" />
                                        )}
                                        Install
                                    </button>
                                </div>
                                <div className="flex items-center gap-2 p-3 rounded-lg bg-white/5 border border-white/5">
                                    <Info className="w-3.5 h-3.5 text-muted-foreground" />
                                    <p className="text-[10px] text-muted-foreground">
                                        The gateway will automatically hot-reload and list any new skills found in the repository.
                                    </p>
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            <div className="flex-1 overflow-y-auto px-8 pb-8 scrollbar-hide">
                <div className="max-w-6xl mx-auto">
                    {isLoading && skills.length === 0 ? (
                        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                            {[1, 2, 3, 4, 5, 6].map(i => (
                                <div key={i} className="h-44 rounded-2xl border border-white/5 bg-white/[0.02] animate-pulse" />
                            ))}
                        </div>
                    ) : filteredSkills.length > 0 ? (
                        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6 pb-4">
                            <AnimatePresence mode="popLayout">
                                {filteredSkills.map((skill) => (
                                    <motion.div
                                        key={skill.skillKey}
                                        layout
                                        initial={{ opacity: 0, scale: 0.95 }}
                                        animate={{ opacity: 1, scale: 1 }}
                                        exit={{ opacity: 0, scale: 0.95 }}
                                    >
                                        <SkillCard skill={skill} onToggle={handleToggle} />
                                    </motion.div>
                                ))}
                            </AnimatePresence>
                        </div>
                    ) : (
                        <div className="py-20 flex flex-col items-center justify-center text-center space-y-4">
                            <div className="p-4 rounded-full bg-white/5 border border-white/10">
                                <Package className="w-8 h-8 text-muted-foreground" />
                            </div>
                            <div>
                                <h3 className="text-lg font-semibold">No skills matching "{search}"</h3>
                                <p className="text-sm text-muted-foreground">Try a different search term or clear the filter.</p>
                            </div>
                        </div>
                    )}

                    {/* Advanced Info */}
                    <div className="mt-8 p-6 rounded-2xl border bg-primary/5 border-primary/10 flex gap-4">
                        <div className="p-2 bg-primary/10 rounded-xl h-fit">
                            <Shield className="w-5 h-5 text-primary" />
                        </div>
                        <div>
                            <h4 className="text-sm font-semibold text-primary uppercase tracking-wider">Modular Architecture</h4>
                            <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                                Skills are dynamic toolsets that can be hot-reloaded on the OpenClaw node.
                                Disabling a skill immediately removes its associated tools from the agent's available registry for subsequent runs.
                            </p>
                        </div>
                    </div>
                </div>
            </div>
        </motion.div>
    );
}
