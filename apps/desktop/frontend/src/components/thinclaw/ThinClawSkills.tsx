import { motion, AnimatePresence } from 'framer-motion';
import { RefreshCw, Search, Shield, Zap, Package, FolderGit2, Plus, Info, Upload } from 'lucide-react';
import { cn } from '../../lib/utils';
import { SkillCard } from './skills/SkillCard';
import { useSkills } from './skills/use-skills';

export function ThinClawSkills() {
    const {
        skills,
        filteredSkills,
        isLoading,
        search,
        setSearch,
        showMarketplace,
        setShowMarketplace,
        repoUrl,
        setRepoUrl,
        isInstalling,
        gatewayMode,
        catalogQuery,
        setCatalogQuery,
        catalogResults,
        catalogSearching,
        inspectName,
        setInspectName,
        inspectResult,
        publishName,
        setPublishName,
        publishRepo,
        setPublishRepo,
        publishResult,
        setPublishResult,
        handleInstallRepo,
        handleCatalogSearch,
        handleInstallSkill,
        handleReloadAll,
        handleInspect,
        handleReload,
        handleRemove,
        handleTrust,
        handlePublish
    } = useSkills();
    const activeCount = skills.filter((skill) => !skill.disabled && skill.eligible).length;
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
                                className="pl-9 pr-4 py-2 rounded-xl bg-card border border-border/40 focus:border-primary/50 focus:ring-1 focus:ring-primary/50 transition-all text-sm w-64 shadow-inner"
                            />
                        </div>
                        <button
                            onClick={() => setShowMarketplace(!showMarketplace)}
                            className={cn(
                                'flex items-center gap-2 px-4 py-2 rounded-xl border transition-all text-sm font-bold shadow-xs',
                                showMarketplace
                                    ? 'bg-primary text-primary-foreground border-primary'
                                    : 'bg-card border-border/40 hover:bg-white/5'
                            )}
                            title={
                                gatewayMode === 'remote'
                                    ? 'Uses the remote ThinClaw gateway skill API'
                                    : 'Uses the local ThinClaw skill registry'
                            }
                        >
                            <Plus className="w-4 h-4" />
                            Add Skills
                        </button>
                        <div className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary flex items-center gap-2 text-sm font-bold shadow-lg shadow-primary/5">
                            <Zap className="w-4 h-4 fill-current" />
                            {activeCount} / {totalCount} active
                        </div>
                        <button
                            onClick={handleReloadAll}
                            className="p-2.5 rounded-xl bg-card border border-border/40 hover:bg-white/5 transition-colors shadow-xs"
                            title="Reload all skills"
                        >
                            <RefreshCw className={cn('w-4 h-4', isLoading && 'animate-spin')} />
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
                            <div className="p-6 rounded-2xl border bg-card border-border/40 space-y-5 shadow-2xl">
                                <div className="space-y-3">
                                    <div className="flex items-center gap-3">
                                        <div className="p-2 bg-primary/10 rounded-lg">
                                            <Search className="w-5 h-5 text-primary" />
                                        </div>
                                        <div>
                                            <h3 className="font-semibold text-sm">Search Skill Catalog</h3>
                                            <p className="text-xs text-muted-foreground">
                                                Install catalog skills through the ThinClaw skill registry.
                                            </p>
                                        </div>
                                    </div>
                                    <div className="flex gap-3">
                                        <input
                                            type="text"
                                            placeholder="Search by skill name or capability"
                                            value={catalogQuery}
                                            onChange={(e) => setCatalogQuery(e.target.value)}
                                            onKeyDown={(e) => e.key === 'Enter' && handleCatalogSearch()}
                                            className="flex-1 px-4 py-2.5 rounded-xl bg-white/5 border border-border/40 focus:border-primary/50 focus:ring-1 focus:ring-primary/50 transition-all text-sm"
                                        />
                                        <button
                                            onClick={handleCatalogSearch}
                                            disabled={catalogSearching || !catalogQuery.trim()}
                                            className="px-5 py-2.5 rounded-xl bg-primary/15 text-primary text-sm font-bold border border-primary/20 hover:bg-primary/20 transition-all disabled:opacity-50 flex items-center gap-2"
                                        >
                                            {catalogSearching ? (
                                                <RefreshCw className="w-4 h-4 animate-spin" />
                                            ) : (
                                                <Search className="w-4 h-4" />
                                            )}
                                            Search
                                        </button>
                                    </div>
                                    {catalogResults.length > 0 && (
                                        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                                            {catalogResults.slice(0, 6).map((entry: any) => (
                                                <div
                                                    key={entry.slug || entry.name}
                                                    className="p-3 rounded-xl bg-white/3 border border-white/5 flex items-start gap-3"
                                                >
                                                    <Package className="w-4 h-4 text-primary mt-0.5 shrink-0" />
                                                    <div className="flex-1 min-w-0">
                                                        <div className="flex items-center gap-2">
                                                            <p className="text-sm font-semibold truncate">
                                                                {entry.name || entry.slug}
                                                            </p>
                                                            {entry.version && (
                                                                <span className="text-[10px] text-muted-foreground">
                                                                    v{entry.version}
                                                                </span>
                                                            )}
                                                        </div>
                                                        <p className="text-xs text-muted-foreground line-clamp-2 mt-1">
                                                            {entry.description}
                                                        </p>
                                                        <div className="flex items-center gap-3 mt-2 text-[10px] text-muted-foreground">
                                                            {entry.owner && <span>{entry.owner}</span>}
                                                            {entry.stars !== undefined && (
                                                                <span>{entry.stars} stars</span>
                                                            )}
                                                            {entry.downloads !== undefined && (
                                                                <span>{entry.downloads} downloads</span>
                                                            )}
                                                        </div>
                                                    </div>
                                                    <button
                                                        onClick={() => handleInstallSkill(entry)}
                                                        disabled={isInstalling}
                                                        className="p-2 rounded-lg bg-primary/15 text-primary border border-primary/20 hover:bg-primary/20 transition-colors"
                                                        title="Install"
                                                    >
                                                        <Plus className="w-4 h-4" />
                                                    </button>
                                                </div>
                                            ))}
                                        </div>
                                    )}
                                </div>
                                <div className="h-px bg-border/40" />
                                <div className="flex items-center gap-3">
                                    <div className="p-2 bg-primary/10 rounded-lg">
                                        <FolderGit2 className="w-5 h-5 text-primary" />
                                    </div>
                                    <div>
                                        <h3 className="font-semibold text-sm">Install Skill Repository</h3>
                                        <p className="text-xs text-muted-foreground">
                                            Clone a collection of tools directly from GitHub into your workspace.
                                        </p>
                                    </div>
                                </div>
                                <div className="flex gap-3">
                                    <input
                                        type="text"
                                        placeholder="https://github.com/thinclaw/skills"
                                        value={repoUrl}
                                        onChange={(e) => setRepoUrl(e.target.value)}
                                        className="flex-1 px-4 py-2.5 rounded-xl bg-white/5 border border-border/40 focus:border-primary/50 focus:ring-1 focus:ring-primary/50 transition-all text-sm"
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
                                        The gateway will automatically hot-reload and list any new skills found in the
                                        repository.
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
                            {[1, 2, 3, 4, 5, 6].map((i) => (
                                <div
                                    key={i}
                                    className="h-44 rounded-2xl border border-white/5 bg-white/2 animate-pulse"
                                />
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
                                        <SkillCard
                                            skill={skill}
                                            onInspect={handleInspect}
                                            onReload={handleReload}
                                            onRemove={handleRemove}
                                            onTrust={handleTrust}
                                            onPublish={(name) => {
                                                setPublishName(name);
                                                setPublishResult(null);
                                            }}
                                        />
                                    </motion.div>
                                ))}
                            </AnimatePresence>
                        </div>
                    ) : (
                        <div className="py-20 flex flex-col items-center justify-center text-center space-y-4">
                            <div className="p-4 rounded-full bg-white/5 border border-border/40">
                                <Package className="w-8 h-8 text-muted-foreground" />
                            </div>
                            <div>
                                <h3 className="text-lg font-semibold">No skills matching "{search}"</h3>
                                <p className="text-sm text-muted-foreground">
                                    Try a different search term or clear the filter.
                                </p>
                            </div>
                        </div>
                    )}

                    {/* Advanced Info */}
                    <div className="mt-8 p-6 rounded-2xl border bg-primary/5 border-primary/10 flex gap-4">
                        <div className="p-2 bg-primary/10 rounded-xl h-fit">
                            <Shield className="w-5 h-5 text-primary" />
                        </div>
                        <div>
                            <h4 className="text-sm font-semibold text-primary uppercase tracking-wider">
                                Modular Architecture
                            </h4>
                            <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                                Skills are dynamic toolsets that can be hot-reloaded on the ThinClaw node. Disabling a
                                skill immediately removes its associated tools from the agent's available registry for
                                subsequent runs.
                            </p>
                        </div>
                    </div>

                    <AnimatePresence>
                        {inspectName && (
                            <motion.div
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                                className="fixed inset-0 z-50 bg-black/60 backdrop-blur-xs flex items-center justify-center p-6"
                                onClick={() => setInspectName(null)}
                            >
                                <div
                                    className="w-full max-w-4xl max-h-[80vh] overflow-hidden rounded-2xl border border-border bg-background shadow-2xl"
                                    onClick={(e) => e.stopPropagation()}
                                >
                                    <div className="p-4 border-b border-border flex items-center justify-between">
                                        <div>
                                            <h3 className="font-semibold">Inspect {inspectName}</h3>
                                            <p className="text-xs text-muted-foreground">
                                                Manifest, package files, and audit findings.
                                            </p>
                                        </div>
                                        <button
                                            className="text-xs text-muted-foreground hover:text-foreground"
                                            onClick={() => setInspectName(null)}
                                        >
                                            Close
                                        </button>
                                    </div>
                                    <pre className="p-4 overflow-auto max-h-[65vh] text-xs font-mono whitespace-pre-wrap">
                                        {inspectResult ? JSON.stringify(inspectResult, null, 2) : 'Loading...'}
                                    </pre>
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>

                    <AnimatePresence>
                        {publishName && (
                            <motion.div
                                initial={{ opacity: 0 }}
                                animate={{ opacity: 1 }}
                                exit={{ opacity: 0 }}
                                className="fixed inset-0 z-50 bg-black/60 backdrop-blur-xs flex items-center justify-center p-6"
                                onClick={() => setPublishName(null)}
                            >
                                <div
                                    className="w-full max-w-3xl max-h-[80vh] overflow-hidden rounded-2xl border border-border bg-background shadow-2xl"
                                    onClick={(e) => e.stopPropagation()}
                                >
                                    <div className="p-4 border-b border-border flex items-center justify-between">
                                        <div>
                                            <h3 className="font-semibold">Publish {publishName}</h3>
                                            <p className="text-xs text-muted-foreground">
                                                Dry-run package and policy validation before opening a remote PR.
                                            </p>
                                        </div>
                                        <button
                                            className="text-xs text-muted-foreground hover:text-foreground"
                                            onClick={() => setPublishName(null)}
                                        >
                                            Close
                                        </button>
                                    </div>
                                    <div className="p-4 space-y-4">
                                        <div className="flex gap-3">
                                            <input
                                                value={publishRepo}
                                                onChange={(e) => setPublishRepo(e.target.value)}
                                                placeholder="owner/repo"
                                                className="flex-1 px-4 py-2.5 rounded-xl bg-white/5 border border-border/40 focus:border-primary/50 focus:ring-1 focus:ring-primary/50 transition-all text-sm font-mono"
                                            />
                                            <button
                                                onClick={handlePublish}
                                                disabled={!publishRepo.trim()}
                                                className="px-5 py-2.5 rounded-xl bg-primary text-primary-foreground text-sm font-bold disabled:opacity-50 flex items-center gap-2"
                                            >
                                                <Upload className="w-4 h-4" />
                                                Dry Run
                                            </button>
                                        </div>
                                        <pre className="p-3 rounded-xl bg-white/3 border border-white/5 overflow-auto max-h-[48vh] text-xs font-mono whitespace-pre-wrap">
                                            {publishResult
                                                ? JSON.stringify(publishResult, null, 2)
                                                : 'No publish dry-run yet.'}
                                        </pre>
                                    </div>
                                </div>
                            </motion.div>
                        )}
                    </AnimatePresence>
                </div>
            </div>
        </motion.div>
    );
}
