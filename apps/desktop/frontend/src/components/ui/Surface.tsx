import { forwardRef, type HTMLAttributes } from "react";
import { cn } from "../../lib/utils";

export interface SurfaceProps extends HTMLAttributes<HTMLDivElement> {
    elevation?: "panel" | "elevated" | "subtle";
}

const elevations = {
    panel: "bg-surface-panel",
    elevated: "bg-surface-elevated shadow-lg",
    subtle: "bg-surface-subtle",
} as const;

export const Surface = forwardRef<HTMLDivElement, SurfaceProps>(function Surface(
    { className, elevation = "panel", ...props },
    ref,
) {
    return (
        <div
            ref={ref}
            className={cn(
                "rounded-[var(--radius-panel)] border border-surface-outline text-content-primary",
                elevations[elevation],
                className,
            )}
            {...props}
        />
    );
});
