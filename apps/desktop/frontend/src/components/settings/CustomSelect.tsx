import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronDown, Zap, AlertTriangle } from 'lucide-react';
import { cn } from '../../lib/utils';

const OptimizedIcon = () => (
    <div className="relative flex items-center justify-center w-4 h-4">
        <div className="absolute inset-0 bg-emerald-500/20 rounded-full animate-pulse" />
        <Zap className="w-3 h-3 text-emerald-500 fill-emerald-500/20" />
    </div>
);

const RiskIcon = () => (
    <div className="relative flex items-center justify-center w-4 h-4">
        <div className="absolute inset-0 bg-amber-500/20 rounded-full animate-ping opacity-20" />
        <AlertTriangle className="w-3 h-3 text-amber-500" />
    </div>
);

export function CustomSelect({
    value,
    onChange,
    options,
    disabled,
    placeholder = "Select option..."
}: {
    value: number,
    onChange: (val: number) => void,
    options: { value: number, label: string, disabled?: boolean, risk?: "Safe" | "Moderate" | "Critical" }[],
    disabled?: boolean,
    placeholder?: string
}) {
    const [isOpen, setIsOpen] = useState(false);
    const selectedOption = options.find(o => o.value === value);

    // Close on click outside
    useEffect(() => {
        if (!isOpen) return;
        const handleClick = () => setIsOpen(false);
        window.addEventListener('click', handleClick);
        return () => window.removeEventListener('click', handleClick);
    }, [isOpen]);

    return (
        <div className="relative w-[220px]" onClick={e => e.stopPropagation()}>
            <button
                type="button"
                onClick={() => !disabled && setIsOpen(!isOpen)}
                disabled={disabled}
                className={cn(
                    "flex h-11 w-full items-center justify-between rounded-xl border bg-background/50 px-4 py-2 text-sm shadow-sm transition-all duration-200 backdrop-blur-md",
                    isOpen ? "border-primary ring-2 ring-primary/20 shadow-lg" : "border-border/50 hover:border-border",
                    disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"
                )}
            >
                <span className="truncate font-medium">
                    {selectedOption ? selectedOption.label : placeholder}
                </span>
                <ChevronDown className={cn("h-4 w-4 text-muted-foreground transition-transform duration-300", isOpen && "rotate-180")} />
            </button>

            <AnimatePresence>
                {isOpen && (
                    <motion.div
                        initial={{ opacity: 0, scale: 0.95, y: -10 }}
                        animate={{ opacity: 1, scale: 1, y: 0 }}
                        exit={{ opacity: 0, scale: 0.95, y: -10 }}
                        transition={{ duration: 0.15, ease: "easeOut" }}
                        className="absolute right-0 top-[calc(100%+8px)] z-50 w-full min-w-[200px] overflow-hidden rounded-xl border border-border/50 bg-card/90 p-1.5 shadow-2xl backdrop-blur-xl"
                    >
                        <div className="max-h-[300px] overflow-y-auto custom-scrollbar">
                            {options.map((option) => (
                                <button
                                    key={option.value}
                                    type="button"
                                    disabled={option.disabled}
                                    onClick={() => {
                                        onChange(option.value);
                                        setIsOpen(false);
                                    }}
                                    className={cn(
                                        "flex w-full items-center justify-between rounded-lg px-3 py-2.5 text-left text-sm transition-all duration-200 mb-0.5 last:mb-0",
                                        option.value === value ? "bg-primary/10 text-primary font-bold" : "hover:bg-muted/50 text-foreground",
                                        option.disabled ? "opacity-40 cursor-not-allowed grayscale-[50%]" : "cursor-pointer"
                                    )}
                                >
                                    <span className="flex items-center gap-2">
                                        {option.label}
                                        {option.disabled && <RiskIcon />}
                                    </span>
                                    {option.risk && !option.disabled && (
                                        <div className="flex items-center gap-2">
                                            {option.value < 32768 && <OptimizedIcon />}
                                            <div className={cn(
                                                "w-1.5 h-1.5 rounded-full",
                                                option.risk === "Critical" ? "bg-rose-500 shadow-[0_0_8px_rgba(244,63,94,0.5)]" :
                                                    option.risk === "Moderate" ? "bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.5)]" :
                                                        "bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.5)]"
                                            )} />
                                        </div>
                                    )}
                                </button>
                            ))}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}
