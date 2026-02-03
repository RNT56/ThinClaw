import { useConfig } from '../../hooks/use-config';
import { cn } from '../../lib/utils';
import { Bot, Terminal, Lightbulb, GraduationCap, Palette, Check } from 'lucide-react';
import { motion } from 'framer-motion';

const PERSONAS = [
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

    if (!config) return <div className="p-4 text-center text-muted-foreground">Loading personas...</div>;

    const selectedId = config.selected_persona || 'scrappy';

    const handleSelect = (id: string) => {
        updateConfig({ ...config, selected_persona: id });
    };

    return (
        <div className="space-y-4 animate-in fade-in slide-in-from-bottom-2 duration-300">

            <div className="grid grid-cols-1 gap-3 pr-2">
                {PERSONAS.map((persona) => {
                    const isSelected = selectedId === persona.id;
                    const Icon = persona.icon;

                    return (
                        <motion.button
                            key={persona.id}
                            whileHover={{ scale: 1.01 }}
                            whileTap={{ scale: 0.98 }}
                            onClick={() => handleSelect(persona.id)}
                            className={cn(
                                "relative flex items-center gap-4 p-4 rounded-xl border transition-all text-left group",
                                isSelected
                                    ? cn("bg-card shadow-md", (persona as any).activeBorder)
                                    : "bg-muted/30 border-transparent hover:border-border hover:bg-muted/50"
                            )}
                        >
                            <div className={cn(
                                "w-12 h-12 rounded-lg flex items-center justify-center shrink-0 transition-colors",
                                isSelected ? persona.bgColor : "bg-background"
                            )}>
                                <Icon className={cn("w-6 h-6", isSelected ? persona.color : "text-muted-foreground group-hover:text-foreground")} />
                            </div>

                            <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2">
                                    <span className="font-semibold text-sm">{persona.name}</span>
                                    {isSelected && (
                                        <motion.div
                                            initial={{ opacity: 0, scale: 0.5 }}
                                            animate={{ opacity: 1, scale: 1 }}
                                            className={cn("text-white rounded-full p-0.5", (persona as any).checkBg)}
                                        >
                                            <Check className="w-3 h-3" />
                                        </motion.div>
                                    )}
                                </div>
                                <p className="text-xs text-muted-foreground mt-0.5 leading-relaxed">
                                    {persona.description}
                                </p>
                            </div>

                            {isSelected && (
                                <motion.div
                                    layoutId="persona-active"
                                    className={cn("absolute inset-0 border-2 rounded-xl pointer-events-none", (persona as any).activeBorder)}
                                    transition={{ type: "spring", bounce: 0.2, duration: 0.6 }}
                                />
                            )}
                        </motion.button>
                    );
                })}
            </div>
        </div>
    );
}
