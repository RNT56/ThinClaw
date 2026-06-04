import { useState } from 'react';
import { Loader2, Plus, Key, X } from 'lucide-react';

export function AddSecretForm({ onAdd }: { onAdd: (name: string, value: string, description: string | null) => Promise<void> }) {
    const [name, setName] = useState('');
    const [value, setValue] = useState('');
    const [description, setDescription] = useState('');
    const [loading, setLoading] = useState(false);
    const [isOpen, setIsOpen] = useState(false);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!name.trim() || !value.trim()) return;
        setLoading(true);
        try {
            await onAdd(name.trim(), value.trim(), description.trim() || null);
            setName('');
            setValue('');
            setDescription('');
            setIsOpen(false);
        } catch (e) {
            // Error handled by parent toast
        } finally {
            setLoading(false);
        }
    };

    if (!isOpen) {
        return (
            <button
                onClick={() => setIsOpen(true)}
                className="w-full p-6 border border-dashed border-border/60 rounded-2xl flex items-center justify-center gap-2 text-muted-foreground hover:text-foreground hover:border-primary/50 hover:bg-primary/5 transition-all group"
            >
                <Plus className="w-5 h-5 group-hover:scale-110 transition-transform" />
                <span className="font-bold uppercase tracking-wider text-xs">Add Custom API Secret</span>
            </button>
        );
    }

    return (
        <form onSubmit={handleSubmit} className="p-6 border border-border/50 rounded-2xl bg-card/60 backdrop-blur-md shadow-2xl space-y-6 animate-in fade-in zoom-in-95 duration-200">
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 font-semibold">
                    <Key className="w-4 h-4 text-primary" />
                    Add New Secret
                </div>
                <button type="button" onClick={() => setIsOpen(false)} className="p-1 hover:bg-muted rounded-md">
                    <X className="w-4 h-4 text-muted-foreground" />
                </button>
            </div>

            <div className="grid gap-5">
                <div className="space-y-2">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 ml-1">Secret Name</label>
                    <input
                        autoFocus
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="e.g. OpenAI, ElevenLabs, etc."
                        className="w-full h-11 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                        required
                    />
                </div>
                <div className="space-y-2">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 ml-1">Description (Optional)</label>
                    <input
                        value={description}
                        onChange={(e) => setDescription(e.target.value)}
                        placeholder="What is this key used for?"
                        className="w-full h-11 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>
                <div className="space-y-2">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60 ml-1">Secret Token / Value</label>
                    <input
                        type="password"
                        value={value}
                        onChange={(e) => setValue(e.target.value)}
                        placeholder="Paste your key here"
                        className="w-full h-11 rounded-xl border border-border/50 bg-background/50 px-4 py-2 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                        required
                    />
                </div>
            </div>

            <div className="flex justify-end gap-3 pt-2">
                <button
                    type="button"
                    onClick={() => setIsOpen(false)}
                    className="px-6 h-10 rounded-xl text-xs font-bold uppercase tracking-wider hover:bg-muted transition-colors"
                >
                    Cancel
                </button>
                <button
                    disabled={loading || !name || !value}
                    className="px-6 h-10 rounded-xl bg-primary text-primary-foreground font-bold text-xs uppercase tracking-wider flex items-center gap-2 hover:bg-primary/90 transition-all shadow-sm hover:translate-y-[-1px] disabled:opacity-50 disabled:transform-none"
                >
                    {loading && <Loader2 className="w-4 h-4 animate-spin" />}
                    Save Secret
                </button>
            </div>
        </form>
    );
}
