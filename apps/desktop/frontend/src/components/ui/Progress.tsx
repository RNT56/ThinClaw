import * as ProgressPrimitive from "@radix-ui/react-progress";
import { cn } from "../../lib/utils";

export interface ProgressProps {
    value: number;
    label: string;
    className?: string;
    showValue?: boolean;
}

export function Progress({ value, label, className, showValue = false }: ProgressProps) {
    const bounded = Math.min(100, Math.max(0, value));
    return (
        <div className={cn("space-y-1.5", className)}>
            {showValue && (
                <div className="flex justify-between text-xs text-muted-foreground">
                    <span>{label}</span>
                    <span>{Math.round(bounded)}%</span>
                </div>
            )}
            <ProgressPrimitive.Root
                value={bounded}
                aria-label={label}
                className="relative h-1.5 overflow-hidden rounded-full bg-muted"
            >
                <ProgressPrimitive.Indicator
                    className="h-full bg-primary transition-transform duration-[var(--motion-normal)] ease-out"
                    style={{ transform: `translateX(-${100 - bounded}%)` }}
                />
            </ProgressPrimitive.Root>
        </div>
    );
}
