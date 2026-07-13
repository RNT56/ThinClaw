import { useState } from 'react';
import { motion } from 'framer-motion';
import { Plus, RefreshCw, X } from 'lucide-react';

import { cn } from '../../../lib/utils';

export function CustomHookModal({ isOpen, onClose, onSubmit }: {
    isOpen: boolean;
    onClose: () => void;
    onSubmit: (json: string) => Promise<void>;
}) {
    const [json, setJson] = useState('{\n  "rules": [\n    {\n      "name": "my-custom-hook",\n      "points": ["beforeOutbound"],\n      "append": "\\n\\n— Custom signature"\n    }\n  ]\n}');
    const [isSubmitting, setIsSubmitting] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const handleSubmit = async () => {
        setError(null);
        try {
            JSON.parse(json);
        } catch (e: any) {
            setError(`Invalid JSON: ${e.message}`);
            return;
        }
        setIsSubmitting(true);
        try {
            await onSubmit(json);
            onClose();
        } catch (e: any) {
            setError(e?.toString() || 'Failed to register hook');
        } finally {
            setIsSubmitting(false);
        }
    };

    if (!isOpen) return null;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-xs"
            onClick={onClose}>
            <motion.div
                initial={{ opacity: 0, scale: 0.95 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.95 }}
                className="bg-card border border-border/40 rounded-2xl shadow-2xl w-full max-w-2xl mx-4 overflow-hidden"
                onClick={(e) => e.stopPropagation()}
            >
                <div className="p-6 border-b border-white/5 flex items-center justify-between">
                    <div>
                        <h3 className="text-lg font-bold">Custom Hook</h3>
                        <p className="text-sm text-muted-foreground mt-0.5">
                            Write a custom hook bundle in JSON format.
                        </p>
                    </div>
                    <button onClick={onClose} className="p-2 rounded-lg hover:bg-white/5 text-muted-foreground">
                        <X className="w-4 h-4" />
                    </button>
                </div>
                <div className="p-6">
                    <textarea
                        value={json}
                        onChange={(e) => setJson(e.target.value)}
                        className="w-full h-64 bg-black/30 border border-border/40 rounded-xl p-4 font-mono text-xs text-gray-300 focus:outline-hidden focus:border-primary/50 resize-none"
                        spellCheck={false}
                    />
                    {error && (
                        <div className="mt-3 p-3 rounded-lg bg-red-500/10 border border-red-500/20 text-red-400 text-xs">
                            {error}
                        </div>
                    )}
                </div>
                <div className="p-6 pt-0 flex justify-end gap-3">
                    <button
                        onClick={onClose}
                        className="px-4 py-2 rounded-lg text-sm font-medium bg-white/5 hover:bg-white/10 transition-colors border border-white/5"
                    >
                        Cancel
                    </button>
                    <button
                        onClick={handleSubmit}
                        disabled={isSubmitting}
                        className={cn(
                            "px-4 py-2 rounded-lg text-sm font-bold transition-all",
                            "bg-primary/20 hover:bg-primary/30 text-primary border border-primary/20",
                            isSubmitting && "opacity-50 cursor-not-allowed"
                        )}
                    >
                        {isSubmitting ? (
                            <RefreshCw className="w-4 h-4 animate-spin inline mr-2" />
                        ) : (
                            <Plus className="w-4 h-4 inline mr-2" />
                        )}
                        Register Hook
                    </button>
                </div>
            </motion.div>
        </div>
    );
}
