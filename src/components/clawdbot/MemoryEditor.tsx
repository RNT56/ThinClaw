
import { useState, useEffect } from 'react';
import { Save, RefreshCw, FileText } from 'lucide-react';
import { toast } from 'sonner';
import { commands } from '../../lib/bindings';
import { cn } from '../../lib/utils';

export function MemoryEditor() {
    const [content, setContent] = useState('');
    const [isLoading, setIsLoading] = useState(false);
    const [isSaving, setIsSaving] = useState(false);
    const [isDirty, setIsDirty] = useState(false);
    const [lastSaved, setLastSaved] = useState<Date | null>(null);

    const loadMemory = async () => {
        setIsLoading(true);
        try {
            // @ts-ignore - command might not be in bindings types yet
            const res = await commands.getClawdbotMemory();
            if (res.status === 'ok') {
                setContent(res.data);
                setIsDirty(false);
                setLastSaved(new Date());
            } else {
                toast.error("Failed to load memory: " + res.error);
            }
        } catch (e) {
            console.error(e);
            toast.error("Failed to load memory");
        } finally {
            setIsLoading(false);
        }
    };

    const saveMemory = async () => {
        setIsSaving(true);
        try {
            // @ts-ignore
            const res = await commands.saveClawdbotMemory(content);
            if (res.status === 'ok') {
                setIsDirty(false);
                setLastSaved(new Date());
                toast.success("Memory updated");
            } else {
                toast.error("Failed to save memory: " + res.error);
            }
        } catch (e) {
            console.error(e);
            toast.error("Failed to save memory");
        } finally {
            setIsSaving(false);
        }
    };

    useEffect(() => {
        loadMemory();
    }, []);

    const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
        setContent(e.target.value);
        setIsDirty(true);
    };

    // Keyboard shortcut for save
    const handleKeyDown = (e: React.KeyboardEvent) => {
        if ((e.metaKey || e.ctrlKey) && e.key === 's') {
            e.preventDefault();
            saveMemory();
        }
    };

    return (
        <div className="flex flex-col h-full bg-[#0d1117] text-gray-300 font-mono text-sm">
            {/* Toolbar */}
            <div className="flex items-center justify-between px-4 py-2 border-b border-white/10 bg-black/20">
                <div className="flex items-center gap-2">
                    <FileText className="w-4 h-4 text-purple-400" />
                    <span className="font-bold text-xs uppercase tracking-wider text-gray-400">
                        MEMORY.md
                    </span>
                    {isDirty && (
                        <span className="text-[10px] text-amber-500 bg-amber-500/10 px-1.5 py-0.5 rounded border border-amber-500/20">
                            Unsaved
                        </span>
                    )}
                </div>
                <div className="flex items-center gap-2">
                    {lastSaved && (
                        <span className="text-[10px] text-gray-600 mr-2">
                            Synced {lastSaved.toLocaleTimeString()}
                        </span>
                    )}
                    <button
                        onClick={loadMemory}
                        disabled={isLoading || isDirty} // Prevent overwrite if dirty? Or warn.
                        className="p-1.5 hover:bg-white/10 rounded-md text-gray-400 transition-colors disabled:opacity-50"
                        title="Reload from Disk"
                    >
                        <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                    </button>
                    <button
                        onClick={saveMemory}
                        disabled={isSaving || !isDirty}
                        className={cn(
                            "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-bold transition-all",
                            isDirty
                                ? "bg-purple-600 hover:bg-purple-500 text-white shadow-lg shadow-purple-900/20"
                                : "bg-white/5 text-gray-500 cursor-not-allowed"
                        )}
                        title="Save (Cmd+S)"
                    >
                        {isSaving ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <Save className="w-3.5 h-3.5" />}
                        {isSaving ? "Saving..." : "Save"}
                    </button>
                </div>
            </div>

            {/* Editor Area */}
            <div className="flex-1 relative">
                {isLoading && !content ? (
                    <div className="absolute inset-0 flex items-center justify-center text-gray-600">
                        <RefreshCw className="w-8 h-8 animate-spin opacity-20" />
                    </div>
                ) : (
                    <textarea
                        value={content}
                        onChange={handleChange}
                        onKeyDown={handleKeyDown}
                        className="w-full h-full bg-transparent resize-none p-4 focus:outline-none font-mono text-xs leading-relaxed text-gray-300 selection:bg-purple-500/30"
                        spellCheck={false}
                        placeholder="// Agent memory is empty..."
                    />
                )}
            </div>

            {/* Footer / Status */}
            <div className="px-4 py-1.5 border-t border-white/5 bg-black/40 text-[10px] text-gray-600 flex justify-between">
                <span>Markdown Supported</span>
                <span>{content.length} chars</span>
            </div>
        </div>
    );
}
