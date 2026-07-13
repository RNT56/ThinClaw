import { AlertCircle, Inbox, LoaderCircle } from "lucide-react";
import { type ReactNode } from "react";
import { cn } from "../../lib/utils";
import { Button } from "./Button";

type AsyncStateKind = "loading" | "empty" | "error";

export interface AsyncStateProps {
    kind: AsyncStateKind;
    title: string;
    description?: string;
    actionLabel?: string;
    onAction?: () => void;
    compact?: boolean;
    icon?: ReactNode;
    className?: string;
}

const defaultIcons: Record<AsyncStateKind, ReactNode> = {
    loading: <LoaderCircle className="size-5 animate-spin" aria-hidden="true" />,
    empty: <Inbox className="size-5" aria-hidden="true" />,
    error: <AlertCircle className="size-5" aria-hidden="true" />,
};

export function AsyncState({
    kind,
    title,
    description,
    actionLabel,
    onAction,
    compact = false,
    icon,
    className,
}: AsyncStateProps) {
    const liveRole = kind === "error" ? "alert" : "status";
    return (
        <div
            role={liveRole}
            aria-live={kind === "error" ? "assertive" : "polite"}
            className={cn(
                "flex flex-col items-center justify-center text-center text-muted-foreground",
                compact ? "gap-2 p-4" : "min-h-48 gap-3 p-8",
                className,
            )}
        >
            <div className={cn(
                "grid size-10 place-items-center rounded-full bg-muted",
                kind === "error" && "text-destructive",
            )}>
                {icon ?? defaultIcons[kind]}
            </div>
            <div>
                <p className="text-sm font-medium text-foreground">{title}</p>
                {description && <p className="mt-1 max-w-md text-xs leading-relaxed">{description}</p>}
            </div>
            {actionLabel && onAction && (
                <Button size="sm" onClick={onAction}>{actionLabel}</Button>
            )}
        </div>
    );
}
