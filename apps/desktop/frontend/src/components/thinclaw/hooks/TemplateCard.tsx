import { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Check, Copy, Eye, Plus, RefreshCw } from 'lucide-react';

import { cn } from '../../../lib/utils';
import type { HookTemplate } from './templates';

export function TemplateCard({ template, onActivate, isActive }: { template: HookTemplate; onActivate: (t: HookTemplate) => void; isActive: boolean }) {
    const [showPreview, setShowPreview] = useState(false);
    const [copied, setCopied] = useState(false);
    const [activating, setActivating] = useState(false);

    const handleCopy = (e: React.MouseEvent) => {
        e.stopPropagation();
        navigator.clipboard.writeText(JSON.stringify(template.bundle, null, 2));
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    const handleActivate = async (e: React.MouseEvent) => {
        e.stopPropagation();
        setActivating(true);
        try {
            await onActivate(template);
        } finally {
            setActivating(false);
        }
    };

    return (
        <motion.div
            layout
            className={cn(
                "rounded-xl border transition-all duration-200 group",
                "bg-white/2 border-white/5 hover:border-white/15",
                "hover:shadow-lg hover:shadow-primary/5"
            )}
        >
            <div className="p-4">
                <div className="flex items-start gap-3">
                    <div className={cn("p-2 rounded-lg bg-white/5 border border-border/40", template.color)}>
                        {template.icon}
                    </div>
                    <div className="flex-1 min-w-0">
                        <h4 className="text-sm font-semibold">{template.name}</h4>
                        <p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">{template.description}</p>
                    </div>
                </div>

                <div className="flex items-center gap-2 mt-3">
                    {isActive ? (
                        <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold bg-green-500/10 text-green-400 border border-green-500/20">
                            <Check className="w-3 h-3" />
                            Active
                        </div>
                    ) : (
                        <button
                            onClick={handleActivate}
                            disabled={activating}
                            className={cn(
                                "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold transition-all",
                                "bg-primary/10 hover:bg-primary/20 text-primary border border-primary/20",
                                activating && "opacity-50 cursor-not-allowed"
                            )}
                        >
                            {activating ? (
                                <RefreshCw className="w-3 h-3 animate-spin" />
                            ) : (
                                <Plus className="w-3 h-3" />
                            )}
                            Activate
                        </button>
                    )}
                    <button
                        onClick={(e) => { e.stopPropagation(); setShowPreview(!showPreview); }}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-all bg-white/5 hover:bg-white/10 text-muted-foreground border border-white/5"
                    >
                        <Eye className="w-3 h-3" />
                        Preview
                    </button>
                    <button
                        onClick={handleCopy}
                        className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-xs font-medium transition-all bg-white/5 hover:bg-white/10 text-muted-foreground border border-white/5"
                        title="Copy JSON"
                    >
                        {copied ? <Check className="w-3 h-3 text-green-400" /> : <Copy className="w-3 h-3" />}
                    </button>
                </div>
            </div>

            <AnimatePresence>
                {showPreview && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="px-4 pb-4 border-t border-white/5 pt-3">
                            <pre className="text-[10px] font-mono text-muted-foreground bg-black/30 rounded-lg p-3 overflow-x-auto whitespace-pre-wrap border border-white/5">
                                {JSON.stringify(template.bundle, null, 2)}
                            </pre>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}
