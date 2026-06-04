import { useState } from 'react';
import { useConfig } from '../../hooks/use-config';
import { KnowledgeBit } from '../../lib/bindings';
import { directCommands } from '../../lib/generated/direct-commands';
import { Plus, Trash2, Edit2, Lock, Save } from 'lucide-react';
import * as Dialog from "@radix-ui/react-dialog";
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import { v4 as uuidv4 } from 'uuid';

export function PersonalizationTab() {
    const { config, updateConfig } = useConfig();

    // Knowledge Bit State
    const [editingBit, setEditingBit] = useState<KnowledgeBit | null>(null);
    const [isDialogOpen, setIsDialogOpen] = useState(false);

    // Delete Confirmation State
    const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);

    // Form State
    const [formLabel, setFormLabel] = useState("");
    const [formContent, setFormContent] = useState("");

    if (!config) return <div className="p-4 text-center text-muted-foreground">Loading settings...</div>;



    const handleAddBit = () => {
        setEditingBit(null);
        setFormLabel("");
        setFormContent("");
        setIsDialogOpen(true);
    };

    const handleEditBit = (bit: KnowledgeBit) => {
        setEditingBit(bit);
        setFormLabel(bit.label);
        setFormContent(bit.content);
        setIsDialogOpen(true);
    };

    const handleDeleteBit = (id: string) => {
        if (confirm("Delete this knowledge bit?")) {
            const newBits = (config.knowledge_bits || []).filter(b => b.id !== id);
            updateConfig({ ...config, knowledge_bits: newBits });
        }
    };

    const handleToggleBit = (id: string) => {
        const newBits = (config.knowledge_bits || []).map(b =>
            b.id === id ? { ...b, enabled: !b.enabled } : b
        );
        updateConfig({ ...config, knowledge_bits: newBits });
    };

    const saveBit = () => {
        if (!formLabel.trim() || !formContent.trim()) {
            toast.error("Label and Content are required");
            return;
        }

        let newBits = [...(config.knowledge_bits || [])];
        if (editingBit) {
            newBits = newBits.map(b =>
                b.id === editingBit.id ? { ...b, label: formLabel, content: formContent } : b
            );
        } else {
            newBits.push({
                id: uuidv4(),
                label: formLabel,
                content: formContent,
                enabled: true
            });
        }
        updateConfig({ ...config, knowledge_bits: newBits });
        setIsDialogOpen(false);
    };

    return (
        <div className="space-y-10 animate-in fade-in slide-in-from-bottom-4 duration-500">
            {/* General Knowledge Section */}
            <div className="p-8 border rounded-2xl bg-gradient-to-br from-card to-background shadow-xl border-border/30 space-y-8 relative overflow-hidden group">
                <div className="absolute top-0 right-0 w-64 h-64 bg-primary/5 rounded-full blur-3xl -mr-32 -mt-32 transition-colors group-hover:bg-primary/10" />

                <div className="flex items-center justify-between relative z-10">
                    <div className="space-y-1">
                        <div className="flex items-center gap-2">
                            <Save className="w-5 h-5 text-primary" />
                            <h3 className="text-xl font-bold tracking-tight">General Knowledge</h3>
                        </div>
                        <p className="text-sm text-muted-foreground max-w-md leading-relaxed">
                            Personalize your AI further by adding permanent facts, coding styles, or baseline instructions it should always remember.
                        </p>
                    </div>
                    <button
                        onClick={handleAddBit}
                        className="flex items-center gap-2 bg-primary text-primary-foreground px-4 py-2 rounded-xl text-sm font-bold hover:opacity-90 transition-all shadow-lg shadow-primary/20 hover:scale-105 active:scale-95"
                    >
                        <Plus className="w-4 h-4" /> Add Fact
                    </button>
                </div>

                <div className="space-y-3 relative z-10">
                    {(config.knowledge_bits || []).length === 0 ? (
                        <div className="text-sm text-muted-foreground text-center py-12 border-2 border-dashed rounded-2xl bg-muted/20 border-border/50">
                            <div className="p-3 bg-muted/50 rounded-full w-fit mx-auto mb-3">
                                <Lock className="w-6 h-6 opacity-30" />
                            </div>
                            No knowledge bits defined yet.
                        </div>
                    ) : (
                        <div className="grid grid-cols-1 gap-3">
                            {(config.knowledge_bits || []).map(bit => (
                                <div
                                    key={bit.id}
                                    className={cn(
                                        "flex items-center justify-between p-4 rounded-xl border transition-all duration-300 group/bit",
                                        bit.enabled
                                            ? "bg-card/50 border-border/50 hover:border-primary/30 hover:bg-primary/5"
                                            : "bg-muted/30 border-transparent opacity-60 grayscale-[0.5]"
                                    )}
                                >
                                    <div className="flex items-center gap-4 overflow-hidden flex-1">
                                        <div className="relative flex items-center h-6">
                                            <input
                                                type="checkbox"
                                                checked={bit.enabled}
                                                onChange={() => handleToggleBit(bit.id)}
                                                className="peer appearance-none w-5 h-5 rounded-md border-2 border-border checked:bg-primary checked:border-primary transition-all cursor-pointer"
                                            />
                                            <div className="absolute inset-0 flex items-center justify-center pointer-events-none text-white opacity-0 peer-checked:opacity-100 transition-opacity">
                                                <Plus className="w-3 h-3 rotate-45 scale-125" />
                                            </div>
                                        </div>
                                        <div className="flex-1 min-w-0">
                                            <div className="text-sm font-bold truncate group-hover/bit:text-primary transition-colors">{bit.label}</div>
                                            <div className="text-xs text-muted-foreground truncate leading-relaxed">{bit.content}</div>
                                        </div>
                                    </div>
                                    <div className="flex items-center gap-2 pl-4 opacity-0 group-hover/bit:opacity-100 transition-opacity">
                                        <button
                                            onClick={() => handleEditBit(bit)}
                                            className="p-2 hover:bg-emerald-500/10 rounded-lg text-muted-foreground hover:text-emerald-600 dark:hover:text-emerald-400 transition-colors"
                                            title="Edit"
                                        >
                                            <Edit2 className="w-4 h-4" />
                                        </button>
                                        <button
                                            onClick={() => handleDeleteBit(bit.id)}
                                            className="p-2 hover:bg-rose-500/10 rounded-lg text-muted-foreground hover:text-rose-600 dark:hover:text-rose-400 transition-colors"
                                            title="Delete"
                                        >
                                            <Trash2 className="w-4 h-4" />
                                        </button>
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            </div>

            {/* Image Prompt Enhancer Section */}
            <div className="p-8 border rounded-2xl bg-card shadow-lg border-border/30 flex items-center justify-between group">
                <div className="space-y-1">
                    <div className="flex items-center gap-2">
                        <div className="w-2 h-2 rounded-full bg-blue-500 shadow-[0_0_10px_rgba(59,130,246,0.5)]" />
                        <h3 className="text-lg font-bold tracking-tight">Image Prompt Enhancer</h3>
                    </div>
                    <p className="text-sm text-muted-foreground max-w-sm leading-relaxed">
                        Uses a local LLM to automatically expand and refine your image prompts for cinematic results.
                    </p>
                </div>
                <button
                    onClick={() => updateConfig({ ...config, image_prompt_enhance_enabled: !config.image_prompt_enhance_enabled })}
                    className={cn(
                        "relative inline-flex h-7 w-12 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
                        config.image_prompt_enhance_enabled ? "bg-primary" : "bg-muted"
                    )}
                >
                    <span
                        className={cn(
                            "pointer-events-none block h-6 w-6 rounded-full bg-background shadow-lg ring-0 transition-transform duration-300 ease-in-out",
                            config.image_prompt_enhance_enabled ? "translate-x-5" : "translate-x-0"
                        )}
                    />
                </button>
            </div>

            {/* Danger Zone Section */}
            <div className="p-8 border border-rose-500/20 rounded-2xl bg-rose-500/[0.03] space-y-6">
                <div className="flex items-start gap-4">
                    <div className="p-3 bg-rose-500/10 rounded-2xl">
                        <Trash2 className="w-6 h-6 text-rose-600 dark:text-rose-400" />
                    </div>
                    <div className="space-y-1">
                        <h3 className="text-lg font-bold text-rose-700 dark:text-rose-400 tracking-tight">Critical Actions</h3>
                        <p className="text-sm text-muted-foreground max-w-xl leading-relaxed">
                            These actions are permanent and cannot be undone. Always ensure you have backups of important conversations.
                        </p>
                    </div>
                </div>

                <div className="flex flex-col sm:flex-row gap-4 pt-2">
                    <button
                        onClick={() => setDeleteConfirmOpen(true)}
                        className="bg-rose-600 hover:bg-rose-700 text-white px-6 py-3 rounded-xl text-sm font-bold transition-all shadow-lg shadow-rose-900/10 hover:shadow-rose-900/20 hover:scale-[1.02] active:scale-[0.98] flex items-center justify-center gap-2"
                    >
                        <Trash2 className="w-4 h-4" /> Wipe Entire Database
                    </button>
                </div>
            </div>

            {/* Dialogs remain identical in logic but with slightly refined styling if possible via common classes */}
            {/* Delete Confirmation Dialog */}
            <Dialog.Root open={deleteConfirmOpen} onOpenChange={setDeleteConfirmOpen}>
                <Dialog.Portal>
                    <Dialog.Overlay className="fixed inset-0 bg-black/60 backdrop-blur-sm z-[60] animate-in fade-in duration-300" />
                    <Dialog.Content className="fixed left-[50%] top-[50%] z-[60] grid w-full max-w-md translate-x-[-50%] translate-y-[-50%] gap-6 border bg-card p-8 shadow-2xl rounded-2xl animate-in fade-in zoom-in-95 duration-200">
                        <div className="space-y-3 text-center sm:text-left">
                            <Dialog.Title className="text-2xl font-bold text-red-500 tracking-tight">Erase Everything?</Dialog.Title>
                            <Dialog.Description className="text-sm text-muted-foreground leading-relaxed">
                                This is a destructive operation. All of the following will be <span className="text-red-500 font-bold">permanently deleted</span> from your device:
                                <div className="mt-4 p-4 rounded-xl bg-red-500/5 border border-red-500/10 grid grid-cols-2 gap-x-4 gap-y-2 text-[11px] font-medium text-red-600 dark:text-red-400">
                                    <div className="flex items-center gap-2">● Chat Conversations</div>
                                    <div className="flex items-center gap-2">● Active Projects</div>
                                    <div className="flex items-center gap-2">● Document Index</div>
                                    <div className="flex items-center gap-2">● Generated Images</div>
                                    <div className="flex items-center gap-2">● Vector Memory</div>
                                    <div className="flex items-center gap-2">● Summaries</div>
                                </div>
                            </Dialog.Description>
                        </div>
                        <div className="flex flex-col-reverse sm:flex-row justify-end gap-3 mt-4">
                            <button
                                onClick={() => setDeleteConfirmOpen(false)}
                                className="px-6 py-2.5 text-sm font-bold rounded-xl bg-muted hover:bg-muted/80 transition-colors"
                            >
                                Nevermind
                            </button>
                            <button
                                onClick={async () => {
                                    setDeleteConfirmOpen(false);
                                    const tId = toast.loading("Executing data wipe...");
                                    try {
                                        const res = await directCommands.directHistoryDeleteAllHistory();
                                        if (res.status === "error") {
                                            toast.error("Wipe failed", { id: tId, description: res.error });
                                            return;
                                        }
                                        toast.success("All data erased", { id: tId });
                                        setTimeout(() => window.location.reload(), 1500);
                                    } catch (e) {
                                        toast.error("Wipe failed", { id: tId, description: String(e) });
                                    }
                                }}
                                className="px-6 py-2.5 text-sm font-bold rounded-xl bg-red-500 text-white hover:bg-red-600 transition-all shadow-lg shadow-red-500/20"
                            >
                                Confirm Total Wipe
                            </button>
                        </div>
                    </Dialog.Content>
                </Dialog.Portal>
            </Dialog.Root>

            {/* Edit/Add Dialog */}
            <Dialog.Root open={isDialogOpen} onOpenChange={setIsDialogOpen}>
                <Dialog.Portal>
                    <Dialog.Overlay className="fixed inset-0 bg-black/60 backdrop-blur-sm z-[60] animate-in fade-in duration-300" />
                    <Dialog.Content className="fixed left-[50%] top-[50%] z-[60] grid w-full max-w-lg translate-x-[-50%] translate-y-[-50%] gap-6 border bg-card p-8 shadow-2xl rounded-3xl animate-in fade-in zoom-in-95 duration-200">
                        <div className="space-y-1">
                            <Dialog.Title className="text-2xl font-bold tracking-tight">{editingBit ? "Refine Knowledge" : "Inject Knowledge"}</Dialog.Title>
                            <p className="text-sm text-muted-foreground">Information here is injected into the system prompt for all future sessions.</p>
                        </div>
                        <div className="space-y-5">
                            <div className="space-y-2">
                                <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Context Label</label>
                                <input
                                    value={formLabel}
                                    onChange={(e) => setFormLabel(e.target.value)}
                                    placeholder="e.g. My Coding Style"
                                    className="w-full bg-muted/30 border border-border/50 rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 focus:border-primary/50 outline-none transition-all font-semibold"
                                    autoFocus
                                />
                            </div>
                            <div className="space-y-2">
                                <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Instruction / Fact Content</label>
                                <textarea
                                    value={formContent}
                                    onChange={(e) => setFormContent(e.target.value)}
                                    placeholder="Always format code in TypeScript using functional patterns..."
                                    className="w-full bg-muted/30 border border-border/50 rounded-xl px-4 py-3 text-sm resize-none h-40 focus:ring-2 focus:ring-primary/20 focus:border-primary/50 outline-none transition-all leading-relaxed"
                                />
                            </div>
                        </div>
                        <div className="flex flex-col-reverse sm:flex-row justify-end gap-3 mt-2">
                            <Dialog.Close asChild>
                                <button className="px-6 py-2.5 text-sm font-bold rounded-xl bg-muted hover:bg-muted/80 transition-colors">Cancel</button>
                            </Dialog.Close>
                            <button
                                onClick={saveBit}
                                className="px-8 py-2.5 text-sm font-bold rounded-xl bg-primary text-primary-foreground hover:opacity-90 transition-all shadow-lg shadow-primary/20"
                            >
                                {editingBit ? "Update" : "Inject"}
                            </button>
                        </div>
                    </Dialog.Content>
                </Dialog.Portal>
            </Dialog.Root>
        </div>
    );
}
