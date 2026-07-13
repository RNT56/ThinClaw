import { motion } from "framer-motion";
import { Command, Settings } from "lucide-react";
import { useRef, type KeyboardEvent } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui";
import { ChatModeIcon, ImagineModeIcon, ThinClawModeIcon } from "../icons/ModeIcons";
import { CloudSyncIndicator } from "./CloudSyncIndicator";

export type AppMode = "chat" | "thinclaw" | "imagine" | "settings";
export type ProductMode = Exclude<AppMode, "settings">;

interface ModeNavigatorProps {
    activeMode: AppMode;
    onModeChange: (mode: AppMode) => void;
    onOpenPalette: () => void;
    sidebarOpen: boolean;
    gatewayRunning?: boolean;
}

export const PRODUCT_MODES = [
    {
        id: "chat" as const,
        label: "Workbench",
        description: "Direct chat, projects, and local models",
        shortcut: "⌘1",
        Icon: ChatModeIcon,
    },
    {
        id: "thinclaw" as const,
        label: "Agent Cockpit",
        description: "Sessions, tools, channels, and operations",
        shortcut: "⌘2",
        Icon: ThinClawModeIcon,
    },
    {
        id: "imagine" as const,
        label: "Imagine",
        description: "Create and manage generated images",
        shortcut: "⌘3",
        Icon: ImagineModeIcon,
    },
] as const;

export function ModeNavigator({
    activeMode,
    onModeChange,
    onOpenPalette,
    sidebarOpen,
    gatewayRunning,
}: ModeNavigatorProps) {
    const modeRefs = useRef<Array<HTMLButtonElement | null>>([]);

    const handleModeKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
        let nextIndex: number | null = null;
        if (event.key === "ArrowDown" || event.key === "ArrowRight") {
            nextIndex = (index + 1) % PRODUCT_MODES.length;
        } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
            nextIndex = (index - 1 + PRODUCT_MODES.length) % PRODUCT_MODES.length;
        } else if (event.key === "Home") {
            nextIndex = 0;
        } else if (event.key === "End") {
            nextIndex = PRODUCT_MODES.length - 1;
        }
        if (nextIndex === null) return;
        event.preventDefault();
        const nextMode = PRODUCT_MODES[nextIndex];
        onModeChange(nextMode.id);
        modeRefs.current[nextIndex]?.focus();
    };

    return (
        <nav aria-label="Product modes" className="space-y-2 border-t border-border/50 pt-3">
            <div role="tablist" aria-label="Switch workspace" className="space-y-1">
                {PRODUCT_MODES.map((mode, index) => {
                    const isActive = activeMode === mode.id;
                    const showGatewayStatus = mode.id === "thinclaw" && gatewayRunning;
                    return (
                        <motion.button
                            ref={(node) => { modeRefs.current[index] = node; }}
                            key={mode.id}
                            role="tab"
                            aria-selected={isActive}
                            aria-label={sidebarOpen ? undefined : mode.label}
                            tabIndex={isActive || (activeMode === "settings" && index === 0) ? 0 : -1}
                            onKeyDown={(event) => handleModeKeyDown(event, index)}
                            onClick={() => onModeChange(mode.id)}
                            className={cn(
                                "relative flex min-h-11 items-center rounded-[var(--radius-control)] text-left",
                                "transition-colors duration-[var(--motion-fast)]",
                                sidebarOpen ? "w-full gap-3 px-3" : "mx-auto w-11 justify-center",
                                isActive
                                    ? "bg-accent text-foreground shadow-sm ring-1 ring-primary/20"
                                    : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                            )}
                            title={!sidebarOpen ? mode.label : undefined}
                        >
                            <span className="relative shrink-0">
                                <mode.Icon isActive={isActive} size={22} />
                                {showGatewayStatus && (
                                    <span
                                        aria-label="Agent runtime connected"
                                        className="absolute -right-1 -top-1 size-2.5 rounded-full border-2 border-background bg-emerald-500"
                                    />
                                )}
                            </span>
                            {sidebarOpen && (
                                <span className="min-w-0 flex-1">
                                    <span className="flex items-center justify-between gap-2">
                                        <span className="truncate text-sm font-semibold">{mode.label}</span>
                                        <kbd className="text-[10px] font-normal text-muted-foreground">{mode.shortcut}</kbd>
                                    </span>
                                    <span className="block truncate text-[10px] text-muted-foreground">
                                        {mode.description}
                                    </span>
                                </span>
                            )}
                        </motion.button>
                    );
                })}
            </div>

            <CloudSyncIndicator sidebarOpen={sidebarOpen} />

            <Button
                variant="ghost"
                size={sidebarOpen ? "md" : "icon"}
                onClick={onOpenPalette}
                className={cn(sidebarOpen ? "w-full justify-start px-3" : "mx-auto flex")}
                aria-label="Open command palette"
                title={!sidebarOpen ? "Command palette" : undefined}
            >
                <Command className="size-4" aria-hidden="true" />
                {sidebarOpen && (
                    <>
                        <span className="flex-1 text-left">Commands</span>
                        <kbd className="text-[10px] text-muted-foreground">⌘K</kbd>
                    </>
                )}
            </Button>

            <Button
                variant={activeMode === "settings" ? "secondary" : "ghost"}
                size={sidebarOpen ? "md" : "icon"}
                onClick={() => onModeChange("settings")}
                className={cn(sidebarOpen ? "w-full justify-start px-3" : "mx-auto flex")}
                aria-current={activeMode === "settings" ? "page" : undefined}
                aria-label="Settings"
            >
                <Settings className="size-4" aria-hidden="true" />
                {sidebarOpen && <span>Settings</span>}
            </Button>
        </nav>
    );
}
