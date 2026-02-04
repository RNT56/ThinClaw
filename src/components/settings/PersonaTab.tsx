import { useConfig } from '../../hooks/use-config';
import { cn } from '../../lib/utils';
import {
    Bot,
    Terminal,
    Lightbulb,
    GraduationCap,
    Palette,
    Check,
    Trash2,
    UserPlus,
    X,
    Sparkles
} from 'lucide-react';
import { motion, AnimatePresence } from 'framer-motion';
import { useState } from 'react';
import { CustomPersona } from '../../lib/bindings';

const BUILTIN_PERSONAS = [
    {
        id: 'scrappy',
        name: 'Scrappy',
        description: 'The versatile and insightful companion for all topics.',
        icon: Bot,
        color: 'text-blue-500',
        bgColor: 'bg-blue-500/10',
        borderColor: 'border-blue-500/20',
        activeBorder: 'border-blue-500',
        checkBg: 'bg-blue-500'
    },
    {
        id: 'coder',
        name: 'The Coder',
        description: 'Rigorous, technical, and precise software engineer.',
        icon: Terminal,
        color: 'text-emerald-500',
        bgColor: 'bg-emerald-500/10',
        borderColor: 'border-emerald-500/20',
        activeBorder: 'border-emerald-500',
        checkBg: 'bg-emerald-500'
    },
    {
        id: 'philosopher',
        name: 'The Philosopher',
        description: 'Deep, contemplative, and explores existential inquiries.',
        icon: Lightbulb,
        color: 'text-amber-500',
        bgColor: 'bg-amber-500/10',
        borderColor: 'border-amber-500/20',
        activeBorder: 'border-amber-500',
        checkBg: 'bg-amber-500'
    },
    {
        id: 'teacher',
        name: 'The Teacher',
        description: 'Patient, explanatory, and simplifies complex concepts.',
        icon: GraduationCap,
        color: 'text-purple-500',
        bgColor: 'bg-purple-500/10',
        borderColor: 'border-purple-500/20',
        activeBorder: 'border-purple-500',
        checkBg: 'bg-purple-500'
    },
    {
        id: 'creative',
        name: 'The Creative',
        description: 'Imaginative, expressive, and unconventional thinker.',
        icon: Palette,
        color: 'text-pink-500',
        bgColor: 'bg-pink-500/10',
        borderColor: 'border-pink-500/20',
        activeBorder: 'border-pink-500',
        checkBg: 'bg-pink-500'
    }
];

export function PersonaTab() {
    const { config, updateConfig } = useConfig();
    const [isAdding, setIsAdding] = useState(false);
    const [newPersona, setNewPersona] = useState<Omit<CustomPersona, 'id'>>({
        name: '',
        description: '',
        instructions: ''
    });

    if (!config) return <div className="p-4 text-center text-muted-foreground">Loading personas...</div>;

    const selectedId = config.selected_persona || 'scrappy';
    const customPersonas = config.custom_personas || [];

    const handleSelect = (id: string) => {
        updateConfig({ ...config, selected_persona: id });
    };

    const handleAddPersona = () => {
        if (!newPersona.name || !newPersona.instructions) return;

        const id = `custom-${crypto.randomUUID()}`;
        const updatedCustom = [...customPersonas, { ...newPersona, id }];

        updateConfig({
            ...config,
            custom_personas: updatedCustom,
            selected_persona: id
        });

        setNewPersona({ name: '', description: '', instructions: '' });
        setIsAdding(false);
    };

    const handleRemovePersona = (id: string, e: React.MouseEvent) => {
        e.stopPropagation();
        const updatedCustom = customPersonas.filter(p => p.id !== id);
        let nextSelected = selectedId;

        if (selectedId === id) {
            nextSelected = 'scrappy';
        }

        updateConfig({
            ...config,
            custom_personas: updatedCustom,
            selected_persona: nextSelected
        });
    };

    return (
        <div className="flex flex-col h-[850px] space-y-6">
            {/* Scrollable Persona List */}
            <div className="flex-1 overflow-y-auto pr-4 space-y-8 scrollbar-thin scrollbar-thumb-border scrollbar-track-transparent">

                {/* Built-in Section */}
                <div className="space-y-4">
                    <div className="flex items-center gap-3 px-1">
                        <div className="w-1 h-5 rounded-full bg-blue-500 shadow-[0_0_8px_rgba(59,130,246,0.5)]" />
                        <h3 className="text-sm font-bold text-foreground uppercase tracking-[0.1em]">System Personas</h3>
                    </div>
                    <div className="grid grid-cols-1 gap-3">
                        {BUILTIN_PERSONAS.map((persona) => {
                            const isSelected = selectedId === persona.id;
                            const Icon = persona.icon;

                            return (
                                <motion.button
                                    key={persona.id}
                                    whileHover={{ scale: 1.005 }}
                                    whileTap={{ scale: 0.995 }}
                                    onClick={() => handleSelect(persona.id)}
                                    className={cn(
                                        "relative flex items-center gap-4 p-4 rounded-xl border transition-all text-left group",
                                        isSelected
                                            ? cn("bg-card shadow-lg", persona.activeBorder)
                                            : "bg-muted/10 border-transparent hover:border-border/50 hover:bg-muted/20"
                                    )}
                                >
                                    <div className={cn(
                                        "w-12 h-12 rounded-lg flex items-center justify-center shrink-0 transition-colors",
                                        isSelected ? persona.bgColor : "bg-background shadow-inner"
                                    )}>
                                        <Icon className={cn("w-6 h-6", isSelected ? persona.color : "text-muted-foreground group-hover:text-foreground")} />
                                    </div>

                                    <div className="flex-1 min-w-0">
                                        <div className="flex items-center gap-2">
                                            <span className="font-bold text-sm tracking-tight">{persona.name}</span>
                                            {isSelected && (
                                                <div className={cn("text-white rounded-full p-0.5 shadow-sm", persona.checkBg)}>
                                                    <Check className="w-3 h-3" />
                                                </div>
                                            )}
                                        </div>
                                        <p className="text-xs text-muted-foreground mt-0.5 leading-relaxed">
                                            {persona.description}
                                        </p>
                                    </div>
                                </motion.button>
                            );
                        })}
                    </div>
                </div>

                {/* Custom Section */}
                <div className="space-y-4">
                    <div className="flex items-center gap-3 px-1">
                        <div className="w-1 h-5 rounded-full bg-purple-500 shadow-[0_0_8px_rgba(168,85,247,0.5)]" />
                        <h3 className="text-sm font-bold text-foreground uppercase tracking-[0.1em]">Custom Extensions</h3>
                    </div>

                    <div className="grid grid-cols-1 gap-3">
                        <AnimatePresence mode="popLayout">
                            {customPersonas.map((persona) => {
                                const isSelected = selectedId === persona.id;

                                return (
                                    <motion.div
                                        key={persona.id}
                                        initial={{ opacity: 0, x: -10 }}
                                        animate={{ opacity: 1, x: 0 }}
                                        exit={{ opacity: 0, scale: 0.95 }}
                                        layout
                                        className="relative group"
                                    >
                                        <button
                                            onClick={() => handleSelect(persona.id)}
                                            className={cn(
                                                "w-full flex items-center gap-4 p-4 rounded-xl border transition-all text-left",
                                                isSelected
                                                    ? "bg-gradient-to-br from-card to-background shadow-xl border-primary/50 ring-1 ring-primary/20"
                                                    : "bg-muted/10 border-transparent hover:border-border/50 hover:bg-muted/20"
                                            )}
                                        >
                                            <div className={cn(
                                                "w-12 h-12 rounded-lg flex items-center justify-center shrink-0 transition-colors shadow-inner",
                                                isSelected ? "bg-primary/10" : "bg-background"
                                            )}>
                                                <Sparkles className={cn("w-6 h-6", isSelected ? "text-primary" : "text-muted-foreground")} />
                                            </div>

                                            <div className="flex-1 min-w-0 pr-8">
                                                <div className="flex items-center gap-2">
                                                    <span className="font-bold text-sm tracking-tight">{persona.name}</span>
                                                    {isSelected && (
                                                        <div className="bg-primary text-white rounded-full p-0.5 shadow-sm">
                                                            <Check className="w-3 h-3" />
                                                        </div>
                                                    )}
                                                </div>
                                                <p className="text-xs text-muted-foreground mt-0.5 leading-relaxed line-clamp-2">
                                                    {persona.description || "Experimental custom persona instructions."}
                                                </p>
                                            </div>
                                        </button>

                                        <button
                                            onClick={(e) => handleRemovePersona(persona.id, e)}
                                            className="absolute top-4 right-4 p-2 rounded-lg text-muted-foreground hover:text-destructive hover:bg-destructive/10 opacity-0 group-hover:opacity-100 transition-all backdrop-blur-sm"
                                            title="Permanently remove"
                                        >
                                            <Trash2 className="w-4 h-4" />
                                        </button>
                                    </motion.div>
                                );
                            })}
                        </AnimatePresence>

                        {customPersonas.length === 0 && (
                            <div className="border-2 border-dashed border-border/20 rounded-2xl p-8 text-center bg-muted/5">
                                <p className="text-sm text-muted-foreground">Your custom personas will appear here.</p>
                            </div>
                        )}
                    </div>
                </div>
            </div>

            {/* "Add New Persona" Section - Separated Below */}
            <div className="pt-2">
                {!isAdding ? (
                    <motion.button
                        whileHover={{ scale: 1.01 }}
                        whileTap={{ scale: 0.99 }}
                        onClick={() => setIsAdding(true)}
                        className="w-full p-6 border-2 border-dashed border-primary/20 hover:border-primary/50 rounded-2xl bg-primary/5 hover:bg-primary/10 transition-all group flex flex-col items-center justify-center gap-2"
                    >
                        <div className="p-3 bg-primary/20 rounded-full group-hover:bg-primary group-hover:text-primary-foreground transition-all">
                            <Plus className="w-6 h-6" />
                        </div>
                        <span className="font-bold text-sm text-primary tracking-wide">DESIGN CUSTOM PERSONA</span>
                        <p className="text-xs text-muted-foreground">Define new system instructions and personality markers.</p>
                    </motion.button>
                ) : (
                    <motion.div
                        initial={{ opacity: 0, y: 20 }}
                        animate={{ opacity: 1, y: 0 }}
                        className="p-6 border rounded-2xl bg-card shadow-2xl space-y-5 relative overflow-hidden"
                    >
                        <div className="absolute top-0 right-0 p-4">
                            <button onClick={() => setIsAdding(false)} className="p-1.5 hover:bg-muted rounded-lg transition-colors">
                                <X className="w-4 h-4" />
                            </button>
                        </div>

                        <div className="flex items-center gap-3">
                            <div className="p-2 bg-primary/10 rounded-xl">
                                <UserPlus className="w-5 h-5 text-primary" />
                            </div>
                            <h3 className="font-bold text-lg tracking-tight">New Persona Architecture</h3>
                        </div>

                        <div className="space-y-4">
                            <div className="grid grid-cols-2 gap-4">
                                <div className="space-y-1.5">
                                    <label className="text-[10px] font-bold text-muted-foreground uppercase tracking-wider ml-1">Identity Name</label>
                                    <input
                                        type="text"
                                        placeholder="e.g. Senior Architect"
                                        value={newPersona.name}
                                        onChange={e => setNewPersona({ ...newPersona, name: e.target.value })}
                                        className="w-full bg-muted/10 border-border/50 focus:border-primary focus:ring-1 focus:ring-primary h-11 px-4 rounded-xl transition-all outline-none text-sm placeholder:text-muted-foreground/50"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <label className="text-[10px] font-bold text-muted-foreground uppercase tracking-wider ml-1">Role Description</label>
                                    <input
                                        type="text"
                                        placeholder="Brief objective statement"
                                        value={newPersona.description}
                                        onChange={e => setNewPersona({ ...newPersona, description: e.target.value })}
                                        className="w-full bg-muted/10 border-border/50 focus:border-primary focus:ring-1 focus:ring-primary h-11 px-4 rounded-xl transition-all outline-none text-sm placeholder:text-muted-foreground/50"
                                    />
                                </div>
                            </div>

                            <div className="space-y-1.5">
                                <label className="text-[10px] font-bold text-muted-foreground uppercase tracking-wider ml-1">Base Instructions (System Prompt)</label>
                                <textarea
                                    placeholder="Define behavioral guardrails, tone, and domain expertise..."
                                    value={newPersona.instructions}
                                    onChange={e => setNewPersona({ ...newPersona, instructions: e.target.value })}
                                    rows={4}
                                    className="w-full bg-muted/10 border-border/50 focus:border-primary focus:ring-1 focus:ring-primary p-4 rounded-xl transition-all outline-none text-sm resize-none scrollbar-thin placeholder:text-muted-foreground/50"
                                />
                            </div>

                            <div className="flex gap-3 pt-2">
                                <button
                                    onClick={() => setIsAdding(false)}
                                    className="flex-1 h-12 rounded-xl font-bold text-sm border hover:bg-muted transition-all"
                                >
                                    Cancel
                                </button>
                                <button
                                    onClick={handleAddPersona}
                                    disabled={!newPersona.name || !newPersona.instructions}
                                    className="flex-1 h-12 rounded-xl bg-primary text-primary-foreground font-bold text-sm shadow-xl shadow-primary/20 hover:opacity-95 transition-all disabled:opacity-50 flex items-center justify-center gap-2"
                                >
                                    <Check className="w-4 h-4" /> INITIALIZE PERSONA
                                </button>
                            </div>
                        </div>
                    </motion.div>
                )}
            </div>
        </div>
    );
}

// Helper icons that were missing in previous import if any
function Plus(props: any) {
    return (
        <svg
            {...props}
            xmlns="http://www.w3.org/2000/svg"
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
        >
            <path d="M5 12h14" />
            <path d="M12 5v14" />
        </svg>
    )
}
