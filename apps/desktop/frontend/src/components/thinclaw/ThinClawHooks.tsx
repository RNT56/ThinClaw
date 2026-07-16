import { AnimatePresence, motion } from 'framer-motion';
import { AlertCircle, Anchor, Code2, RefreshCw, Sparkles } from 'lucide-react';

import { cn } from '../../lib/utils';
import { CustomHookModal } from './hooks/CustomHookModal';
import { HookCard } from './hooks/HookCard';
import { TemplateCard } from './hooks/TemplateCard';
import { CATEGORY_LABELS, HOOK_POINT_ICONS, HOOK_POINT_STYLE, HOOK_TEMPLATES } from './hooks/templates';
import { useHooks } from './hooks/use-hooks';

export function ThinClawHooks() {
    const {
        hooks, isLoading, setIsLoading, showCustomModal, setShowCustomModal, activeTab,
        setActiveTab, fetchHooks, handleActivateTemplate, handleRemoveHook, handleCustomSubmit,
        hookPointCounts, templatesByCategory, isTemplateActive,
    } = useHooks();

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 flex flex-col h-full overflow-hidden"
        >
            <div className="p-8 pb-4 space-y-6 flex-none max-w-5xl w-full mx-auto">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                    <div>
                        <h1 className="text-3xl font-bold tracking-tight">Lifecycle Hooks</h1>
                        <p className="text-muted-foreground mt-1">
                            Middleware for your agent — filter, transform, or reject events at any lifecycle point.
                        </p>
                    </div>

                    <div className="flex items-center gap-3">
                        <button
                            onClick={() => setShowCustomModal(true)}
                            className="px-4 py-2 rounded-xl bg-white/5 border border-border/40 hover:bg-white/10 transition-colors text-sm font-medium flex items-center gap-2"
                        >
                            <Code2 className="w-4 h-4" />
                            Custom Hook
                        </button>
                        <div className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary flex items-center gap-2 text-sm font-bold shadow-lg shadow-primary/5">
                            <Anchor className="w-4 h-4" />
                            {hooks.length} active
                        </div>
                        <button
                            onClick={() => {
                                setIsLoading(true);
                                fetchHooks();
                            }}
                            className="p-2.5 rounded-xl bg-card border border-border/40 hover:bg-white/5 transition-colors shadow-xs"
                        >
                            <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                        </button>
                    </div>
                </div>

                {/* Tab Switcher */}
                <div className="flex gap-1 bg-white/3 border border-white/5 rounded-xl p-1">
                    <button
                        onClick={() => setActiveTab('active')}
                        className={cn(
                            "flex-1 px-4 py-2 rounded-lg text-sm font-medium transition-all",
                            activeTab === 'active'
                                ? "bg-white/10 text-white shadow-xs"
                                : "text-muted-foreground hover:text-white hover:bg-white/5"
                        )}
                    >
                        <Anchor className="w-3.5 h-3.5 inline mr-2" />
                        Active Hooks ({hooks.length})
                    </button>
                    <button
                        onClick={() => setActiveTab('templates')}
                        className={cn(
                            "flex-1 px-4 py-2 rounded-lg text-sm font-medium transition-all",
                            activeTab === 'templates'
                                ? "bg-white/10 text-white shadow-xs"
                                : "text-muted-foreground hover:text-white hover:bg-white/5"
                        )}
                    >
                        <Sparkles className="w-3.5 h-3.5 inline mr-2" />
                        Hook Templates ({HOOK_TEMPLATES.length})
                    </button>
                </div>

                {/* Hook point summary (only on active tab) */}
                {activeTab === 'active' && Object.keys(hookPointCounts).length > 0 && (
                    <div className="flex flex-wrap gap-2">
                        {Object.entries(hookPointCounts)
                            .sort(([, a], [, b]) => b - a)
                            .map(([point, count]) => (
                                <div
                                    key={point}
                                    className={cn(
                                        "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium border",
                                        HOOK_POINT_STYLE
                                    )}
                                >
                                    {HOOK_POINT_ICONS[point]}
                                    {point}
                                    <span className="font-bold ml-0.5">× {count}</span>
                                </div>
                            ))}
                    </div>
                )}
            </div>

            <div className="flex-1 overflow-y-auto px-8 pb-8 scrollbar-hide">
                <div className="max-w-5xl mx-auto space-y-3">
                    {/* Active Hooks Tab */}
                    {activeTab === 'active' && (
                        <>
                            {isLoading && hooks.length === 0 ? (
                                <div className="space-y-3">
                                    {[1, 2, 3].map(i => (
                                        <div key={i} className="h-24 rounded-2xl border border-white/5 bg-white/2 animate-pulse" />
                                    ))}
                                </div>
                            ) : hooks.length > 0 ? (
                                <AnimatePresence mode="popLayout">
                                    {hooks.map(hook => (
                                        <HookCard
                                            key={hook.name}
                                            hook={hook}
                                            onRemove={() => handleRemoveHook(hook.name)}
                                        />
                                    ))}
                                </AnimatePresence>
                            ) : (
                                <div className="py-16 flex flex-col items-center justify-center text-center space-y-4">
                                    <div className="p-4 rounded-full bg-white/5 border border-border/40">
                                        <Anchor className="w-8 h-8 text-muted-foreground" />
                                    </div>
                                    <div>
                                        <h3 className="text-lg font-semibold">No active hooks</h3>
                                        <p className="text-sm text-muted-foreground mt-1 max-w-md">
                                            Hooks are middleware that intercept events in the agent pipeline. Browse the{' '}
                                            <button onClick={() => setActiveTab('templates')} className="text-primary hover:underline font-medium">
                                                template gallery
                                            </button>{' '}
                                            to get started, or create a custom hook.
                                        </p>
                                    </div>
                                    <div className="flex gap-3 mt-2">
                                        <button
                                            onClick={() => setActiveTab('templates')}
                                            className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary text-sm font-bold hover:bg-primary/20 transition-colors flex items-center gap-2"
                                        >
                                            <Sparkles className="w-4 h-4" />
                                            Browse Templates
                                        </button>
                                        <button
                                            onClick={() => setShowCustomModal(true)}
                                            className="px-4 py-2 rounded-xl bg-white/5 border border-border/40 text-sm font-medium hover:bg-white/10 transition-colors flex items-center gap-2"
                                        >
                                            <Code2 className="w-4 h-4" />
                                            Write Custom
                                        </button>
                                    </div>
                                </div>
                            )}

                            {/* Info section */}
                            <div className="mt-8 p-6 rounded-2xl border bg-primary/5 border-primary/10 flex gap-4">
                                <div className="p-2 bg-primary/10 rounded-xl h-fit">
                                    <AlertCircle className="w-5 h-5 text-primary" />
                                </div>
                                <div>
                                    <h4 className="text-sm font-semibold text-primary uppercase tracking-wider">Hook Pipeline</h4>
                                    <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                                        Hooks execute in priority order (lower number = runs first). A hook can pass through,
                                        modify content, or reject the event entirely. <strong>Fail-Open</strong> hooks continue on error,
                                        while <strong>Fail-Closed</strong> hooks block the pipeline. Active hooks persist until the engine restarts.
                                    </p>
                                </div>
                            </div>
                        </>
                    )}

                    {/* Templates Tab */}
                    {activeTab === 'templates' && (
                        <div className="space-y-8">
                            {Object.entries(templatesByCategory).map(([category, templates]) => (
                                <div key={category}>
                                    <h3 className="text-sm font-bold uppercase tracking-wider mb-3 flex items-center gap-2 text-muted-foreground">
                                        {CATEGORY_LABELS[category]?.label || category}
                                    </h3>
                                    <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                                        {templates.map(template => (
                                            <TemplateCard
                                                key={template.id}
                                                template={template}
                                                onActivate={handleActivateTemplate}
                                                isActive={isTemplateActive(template)}
                                            />
                                        ))}
                                    </div>
                                </div>
                            ))}

                            {/* Custom hook CTA */}
                            <div className="p-6 rounded-2xl border border-dashed border-border/40 bg-white/1 flex items-center justify-between">
                                <div>
                                    <h4 className="text-sm font-semibold">Need something custom?</h4>
                                    <p className="text-xs text-muted-foreground mt-0.5">
                                        Write your own hook bundle with regex rules, content transforms, or outbound webhooks.
                                    </p>
                                </div>
                                <button
                                    onClick={() => setShowCustomModal(true)}
                                    className="px-4 py-2 rounded-xl bg-white/5 border border-border/40 text-sm font-medium hover:bg-white/10 transition-colors flex items-center gap-2 flex-none"
                                >
                                    <Code2 className="w-4 h-4" />
                                    Write Custom Hook
                                </button>
                            </div>
                        </div>
                    )}
                </div>
            </div>

            <CustomHookModal
                isOpen={showCustomModal}
                onClose={() => setShowCustomModal(false)}
                onSubmit={handleCustomSubmit}
            />
        </motion.div>
    );
}
