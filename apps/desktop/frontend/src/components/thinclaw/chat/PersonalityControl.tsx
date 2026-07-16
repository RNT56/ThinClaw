import { useState } from 'react';
import { Sparkles } from 'lucide-react';

import { cn } from '../../../lib/utils';

const PERSONALITIES = [
    { id: 'concise', label: 'Concise', description: 'Short, tightly edited replies' },
    { id: 'creative', label: 'Creative', description: 'Lateral and experimental' },
    { id: 'technical', label: 'Technical', description: 'Precise and exacting' },
    { id: 'playful', label: 'Playful', description: 'Warm and lightly playful' },
    { id: 'formal', label: 'Formal', description: 'Professional and restrained' },
    { id: 'eli5', label: 'Explain simply', description: 'Simple, concrete explanations' },
] as const;

interface PersonalityControlProps {
    disabled?: boolean;
    onCommand: (command: string) => void;
}

export function PersonalityControl({ disabled = false, onCommand }: PersonalityControlProps) {
    const [open, setOpen] = useState(false);

    const choose = (command: string) => {
        setOpen(false);
        onCommand(command);
    };

    return (
        <div className="relative">
            <button
                type="button"
                aria-label="Session personality"
                aria-haspopup="menu"
                aria-expanded={open}
                disabled={disabled}
                onClick={() => setOpen((current) => !current)}
                className={cn(
                    'rounded-xl border border-transparent p-2 text-muted-foreground transition-all',
                    'hover:bg-muted/50 hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40',
                    open && 'border-primary/30 bg-primary/10 text-primary',
                )}
                title="Choose a temporary personality for this session"
            >
                <Sparkles className="h-4 w-4" />
            </button>

            {open && (
                <div
                    role="menu"
                    aria-label="Session personality options"
                    className="absolute bottom-12 right-0 z-50 w-72 rounded-xl border border-border/50 bg-background/95 p-1.5 shadow-2xl backdrop-blur-xl"
                >
                    <div className="px-2 py-1.5 text-[10px] font-bold uppercase tracking-widest text-muted-foreground">
                        Temporary session tone
                    </div>
                    <button
                        type="button"
                        role="menuitem"
                        onClick={() => choose('/personality')}
                        className="w-full rounded-lg px-2.5 py-2 text-left text-xs hover:bg-muted/60"
                    >
                        <span className="font-semibold">Show current</span>
                        <span className="ml-2 text-[10px] text-muted-foreground">and available choices</span>
                    </button>
                    {PERSONALITIES.map((personality) => (
                        <button
                            key={personality.id}
                            type="button"
                            role="menuitem"
                            onClick={() => choose(`/personality ${personality.id}`)}
                            className="flex w-full items-start justify-between gap-3 rounded-lg px-2.5 py-2 text-left hover:bg-muted/60"
                        >
                            <span className="text-xs font-semibold">{personality.label}</span>
                            <span className="text-right text-[10px] leading-relaxed text-muted-foreground">
                                {personality.description}
                            </span>
                        </button>
                    ))}
                    <div className="my-1 border-t border-border/40" />
                    <button
                        type="button"
                        role="menuitem"
                        onClick={() => choose('/personality clear')}
                        className="w-full rounded-lg px-2.5 py-2 text-left text-xs font-semibold text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                    >
                        Restore base identity
                    </button>
                </div>
            )}
        </div>
    );
}
