import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/utils";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "icon";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
    variant?: ButtonVariant;
    size?: ButtonSize;
}

const variants: Record<ButtonVariant, string> = {
    primary: "bg-primary text-primary-foreground hover:bg-primary/90",
    secondary: "border border-border bg-card text-foreground hover:bg-accent",
    ghost: "text-muted-foreground hover:bg-accent hover:text-foreground",
    danger: "bg-destructive text-destructive-foreground hover:bg-destructive/90",
};

const sizes: Record<ButtonSize, string> = {
    sm: "h-[var(--control-height-compact)] px-3 text-xs",
    md: "h-[var(--control-height)] px-4 text-sm",
    icon: "size-[var(--control-height)]",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
    { className, type = "button", variant = "secondary", size = "md", ...props },
    ref,
) {
    return (
        <button
            ref={ref}
            type={type}
            className={cn(
                "inline-flex shrink-0 items-center justify-center gap-2 rounded-[var(--radius-control)] font-medium",
                "transition-[color,background-color,border-color,box-shadow,transform] duration-[var(--motion-fast)]",
                "disabled:pointer-events-none disabled:opacity-50 active:translate-y-px",
                variants[variant],
                sizes[size],
                className,
            )}
            {...props}
        />
    );
});
