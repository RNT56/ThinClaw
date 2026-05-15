/**
 * ActiveEngineChip — small status badge showing which inference engine is active.
 *
 * Uses the app's design-token system (--primary, --muted, etc.) with per-engine
 * accent colours that harmonise with the active app theme.
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Cpu, Zap } from "lucide-react";
import { cn } from "../../lib/utils";

interface EngineInfo {
    id: string;
    display_name: string;
    hf_tag: string;
}

/**
 * Engine colour map — uses opacity modifiers on the design-token colours so
 * they adapt seamlessly to dark/light mode and all five app-theme palettes.
 *
 * llamacpp is the neutral "default", so it uses the primary token.
 * Others get distinctive but non-jarring tinted variants.
 */
const ENGINE_STYLES: Record<string, { text: string; bg: string; border: string; ring: string }> = {
    llamacpp: {
        text: "text-primary",
        bg: "bg-primary/5",
        border: "border-primary/15",
        ring: "ring-primary/10",
    },
    mlx: {
        text: "text-amber-600 dark:text-amber-400",
        bg: "bg-amber-500/5",
        border: "border-amber-500/15",
        ring: "ring-amber-500/10",
    },
    vllm: {
        text: "text-emerald-600 dark:text-emerald-400",
        bg: "bg-emerald-500/5",
        border: "border-emerald-500/15",
        ring: "ring-emerald-500/10",
    },
    ollama: {
        text: "text-violet-600 dark:text-violet-400",
        bg: "bg-violet-500/5",
        border: "border-violet-500/15",
        ring: "ring-violet-500/10",
    },
};

const FALLBACK_STYLE = {
    text: "text-muted-foreground",
    bg: "bg-muted/30",
    border: "border-border/30",
    ring: "ring-ring/10",
};

export function ActiveEngineChip() {
    const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);

    useEffect(() => {
        invoke<EngineInfo>("direct_runtime_get_active_engine_info")
            .then(setEngineInfo)
            .catch(() => {
                /* silently fail — chip just won't show */
            });
    }, []);

    if (!engineInfo) return null;

    const s = ENGINE_STYLES[engineInfo.id] ?? FALLBACK_STYLE;

    return (
        <div
            className={cn(
                "inline-flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider",
                "px-2.5 py-1 rounded-lg border shadow-sm transition-all duration-200",
                "ring-1",
                s.text,
                s.bg,
                s.border,
                s.ring
            )}
            title={`Inference engine: ${engineInfo.display_name}`}
        >
            {engineInfo.id === "llamacpp" || engineInfo.id === "ollama" ? (
                <Cpu className="w-3 h-3" />
            ) : (
                <Zap className="w-3 h-3" />
            )}
            {engineInfo.display_name}
        </div>
    );
}
