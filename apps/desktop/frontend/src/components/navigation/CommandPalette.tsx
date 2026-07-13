import * as Dialog from "@radix-ui/react-dialog";
import { Bot, Command, Image, MessageSquare, Search, Settings, SlidersHorizontal, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import type { SettingsPage } from "../settings/SettingsSidebar";
import type { ProductMode } from "./ModeNavigator";

interface PaletteCommand {
    id: string;
    label: string;
    description: string;
    keywords: string;
    shortcut?: string;
    icon: typeof MessageSquare;
    run: () => void;
}

interface CommandPaletteProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onModeChange: (mode: ProductMode) => void;
    onSettingsChange: (page: SettingsPage) => void;
}

export function CommandPalette({ open, onOpenChange, onModeChange, onSettingsChange }: CommandPaletteProps) {
    const [query, setQuery] = useState("");
    const inputRef = useRef<HTMLInputElement>(null);

    const commands = useMemo<PaletteCommand[]>(() => {
        const closeAndRun = (action: () => void) => {
            onOpenChange(false);
            action();
        };
        return [
            { id: "workbench", label: "Open Workbench", description: "Direct chat, projects, and local models", keywords: "chat direct local", shortcut: "⌘1", icon: MessageSquare, run: () => closeAndRun(() => onModeChange("chat")) },
            { id: "cockpit", label: "Open Agent Cockpit", description: "Sessions, tools, channels, and operations", keywords: "agent thinclaw runtime dashboard", shortcut: "⌘2", icon: Bot, run: () => closeAndRun(() => onModeChange("thinclaw")) },
            { id: "imagine", label: "Open Imagine", description: "Create and manage generated images", keywords: "image generation media", shortcut: "⌘3", icon: Image, run: () => closeAndRun(() => onModeChange("imagine")) },
            { id: "models", label: "Manage models", description: "Download and select local models", keywords: "settings llm inference", icon: SlidersHorizontal, run: () => closeAndRun(() => onSettingsChange("models")) },
            { id: "appearance", label: "Appearance and language", description: "Theme, density, language, and shortcuts", keywords: "settings theme locale accessibility", icon: Settings, run: () => closeAndRun(() => onSettingsChange("appearance")) },
            { id: "secrets", label: "Manage secrets", description: "API credentials and recovery controls", keywords: "settings keys credentials security", icon: Settings, run: () => closeAndRun(() => onSettingsChange("secrets")) },
        ];
    }, [onModeChange, onOpenChange, onSettingsChange]);

    const normalizedQuery = query.trim().toLowerCase();
    const filtered = commands.filter((command) => !normalizedQuery ||
        `${command.label} ${command.description} ${command.keywords}`.toLowerCase().includes(normalizedQuery));

    useEffect(() => {
        if (open) {
            setQuery("");
            requestAnimationFrame(() => inputRef.current?.focus());
        }
    }, [open]);

    return (
        <Dialog.Root open={open} onOpenChange={onOpenChange}>
            <Dialog.Portal>
                <Dialog.Overlay className="fixed inset-0 z-50 bg-background/70 backdrop-blur-sm data-[state=open]:animate-in data-[state=closed]:animate-out" />
                <Dialog.Content
                    aria-describedby="command-palette-description"
                    className={cn(
                        "fixed left-1/2 top-[15%] z-50 w-[min(38rem,calc(100vw-2rem))] -translate-x-1/2",
                        "overflow-hidden rounded-[var(--radius-dialog)] border border-border bg-popover shadow-2xl",
                        "data-[state=open]:animate-in data-[state=closed]:animate-out",
                    )}
                >
                    <Dialog.Title className="sr-only">Command palette</Dialog.Title>
                    <Dialog.Description id="command-palette-description" className="sr-only">
                        Search actions or switch between ThinClaw product modes.
                    </Dialog.Description>
                    <div className="flex items-center gap-3 border-b border-border px-4">
                        <Search className="size-4 text-muted-foreground" aria-hidden="true" />
                        <input
                            ref={inputRef}
                            value={query}
                            onChange={(event) => setQuery(event.currentTarget.value)}
                            onKeyDown={(event) => {
                                if (event.key === "Enter" && filtered[0]) filtered[0].run();
                            }}
                            placeholder="Search modes and settings…"
                            aria-label="Search commands"
                            className="h-12 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
                        />
                        <Dialog.Close aria-label="Close command palette" className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground">
                            <X className="size-4" aria-hidden="true" />
                        </Dialog.Close>
                    </div>

                    <div className="max-h-[min(24rem,60vh)] overflow-y-auto p-2">
                        {filtered.length === 0 ? (
                            <p role="status" className="px-3 py-8 text-center text-sm text-muted-foreground">No matching commands</p>
                        ) : filtered.map((command) => (
                            <button key={command.id} onClick={command.run} className="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left hover:bg-accent focus-visible:bg-accent">
                                <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
                                    <command.icon className="size-4" aria-hidden="true" />
                                </span>
                                <span className="min-w-0 flex-1">
                                    <span className="block text-sm font-medium text-foreground">{command.label}</span>
                                    <span className="block truncate text-xs text-muted-foreground">{command.description}</span>
                                </span>
                                {command.shortcut && <kbd className="text-xs text-muted-foreground">{command.shortcut}</kbd>}
                            </button>
                        ))}
                    </div>
                    <div className="flex items-center gap-2 border-t border-border px-4 py-2 text-[10px] text-muted-foreground">
                        <Command className="size-3" aria-hidden="true" />
                        <span>Press Enter to run the first match · Escape to close</span>
                    </div>
                </Dialog.Content>
            </Dialog.Portal>
        </Dialog.Root>
    );
}
