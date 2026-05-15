/**
 * ActiveEngineChip — small status badge showing which inference engine is active.
 *
 * Uses the app's design-token system (--primary, --muted, etc.) with per-engine
 * accent colours that harmonise with the active app theme.
 */
import { Cpu, Zap } from "lucide-react";
import { cn } from "../../lib/utils";
import { useModelContext } from "../model-context";

/**
 * Engine colour map — uses opacity modifiers on the design-token colours so
 * they adapt seamlessly to dark/light mode and all five app-theme palettes.
 *
 * llamacpp is the neutral "default", so it uses the primary token.
 * Others get distinctive but non-jarring tinted variants.
 */
const ENGINE_STYLES: Record<string, { text: string; bg: string; border: string; ring: string }> = {
    llama_cpp: {
        text: "text-primary",
        bg: "bg-primary/5",
        border: "border-primary/15",
        ring: "ring-primary/10",
    },
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
    const { engineInfo, runtimeSnapshot } = useModelContext();

    if (!engineInfo && !runtimeSnapshot) return null;

    const engineId = runtimeSnapshot?.kind ?? engineInfo?.id ?? "none";
    const displayName = runtimeSnapshot?.displayName ?? engineInfo?.display_name ?? "Local runtime";
    const readiness = runtimeSnapshot?.readiness ?? "unavailable";
    const s = ENGINE_STYLES[engineId] ?? FALLBACK_STYLE;

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
            title={`Inference engine: ${displayName} (${readiness})`}
        >
            {engineId === "llama_cpp" || engineId === "llamacpp" || engineId === "ollama" ? (
                <Cpu className="w-3 h-3" />
            ) : (
                <Zap className="w-3 h-3" />
            )}
            {displayName}
        </div>
    );
}
