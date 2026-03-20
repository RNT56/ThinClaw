/**
 * EngineSetupBanner — shows when MLX/vLLM needs first-launch bootstrap.
 *
 * Uses the shared `useEngineSetup` hook for setup state and actions.
 * Displays a prominent banner with a "Set Up Now" button that triggers
 * `setup_engine` and shows a progress indicator.
 *
 * Design: follows the app's card pattern (border-border/50, rounded-xl,
 * bg-card, shadow-sm) and uses design-token colours for all states.
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2, Wrench, CheckCircle2, AlertTriangle, Zap } from "lucide-react";
import { cn } from "../../lib/utils";
import { toast } from "sonner";
import { useEngineSetup } from "../../hooks/use-engine-setup";

interface EngineInfo {
    id: string;
    display_name: string;
    hf_tag: string;
}

export function EngineSetupBanner() {
    const {
        status,
        isSettingUp,
        setupStage,
        setupMessage,
        setupComplete,
        setupError,
        triggerSetup,
    } = useEngineSetup();

    const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);

    useEffect(() => {
        invoke<EngineInfo>("get_active_engine_info")
            .then(setEngineInfo)
            .catch((err) => console.warn("Failed to get engine info:", err));
    }, []);

    // Toast on complete/error (hook doesn't do toasts — UI concern)
    useEffect(() => {
        if (setupComplete) toast.success(`${engineInfo?.display_name ?? "Engine"} setup complete!`);
    }, [setupComplete, engineInfo]);

    useEffect(() => {
        if (setupError) toast.error(setupError);
    }, [setupError]);

    // Don't render if no setup needed
    if (!status || (!status.needs_setup && !setupComplete) || !engineInfo) {
        return null;
    }

    // Already set up successfully
    if (setupComplete) {
        return (
            <div className="flex items-center gap-3 p-4 rounded-xl bg-emerald-500/5 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400 text-sm shadow-sm">
                <CheckCircle2 className="w-5 h-5 shrink-0" />
                <div>
                    <span className="font-semibold">{engineInfo.display_name}</span> is ready! You
                    can now discover and load models.
                </div>
            </div>
        );
    }

    return (
        <div
            className={cn(
                "rounded-xl border overflow-hidden transition-all duration-300 shadow-sm",
                setupError
                    ? "bg-card/50 border-rose-500/20"
                    : isSettingUp
                        ? "bg-primary/5 border-primary/20"
                        : "bg-card/50 border-amber-500/20"
            )}
        >
            <div className="p-4 space-y-3">
                {/* Header */}
                <div className="flex items-start gap-3">
                    {isSettingUp ? (
                        <Loader2 className="w-5 h-5 text-primary animate-spin shrink-0 mt-0.5" />
                    ) : setupError ? (
                        <AlertTriangle className="w-5 h-5 text-destructive shrink-0 mt-0.5" />
                    ) : (
                        <Wrench className="w-5 h-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
                    )}
                    <div className="flex-1 min-w-0">
                        <h3 className="font-semibold text-sm text-foreground">
                            {isSettingUp
                                ? `Setting up ${engineInfo.display_name}...`
                                : setupError
                                    ? "Setup Failed"
                                    : `${engineInfo.display_name} Setup Required`}
                        </h3>
                        <p className="text-xs text-muted-foreground mt-1">
                            {isSettingUp
                                ? setupMessage
                                : setupError
                                    ? setupError
                                    : `${engineInfo.display_name} needs a one-time Python environment setup. This downloads and configures the inference runtime (~200MB).`}
                        </p>
                    </div>
                </div>

                {/* Progress bar during setup */}
                {isSettingUp && (
                    <div className="space-y-1.5">
                        <div className="h-1.5 bg-secondary rounded-full overflow-hidden">
                            <div
                                className="h-full bg-primary rounded-full transition-all duration-500 ease-out animate-pulse"
                                style={{
                                    width:
                                        setupStage === "creating_venv"
                                            ? "30%"
                                            : setupStage === "installing"
                                                ? "70%"
                                                : "100%",
                                }}
                            />
                        </div>
                        <div className="flex items-center justify-between text-[10px] text-muted-foreground/60 uppercase tracking-wider">
                            <span
                                className={cn(
                                    "transition-colors",
                                    setupStage === "creating_venv" &&
                                    "text-primary font-semibold"
                                )}
                            >
                                Create Environment
                            </span>
                            <span
                                className={cn(
                                    "transition-colors",
                                    setupStage === "installing" && "text-primary font-semibold"
                                )}
                            >
                                Install Packages
                            </span>
                            <span
                                className={cn(
                                    "transition-colors",
                                    setupStage === "complete" && "text-primary font-semibold"
                                )}
                            >
                                Ready
                            </span>
                        </div>
                    </div>
                )}

                {/* Action button */}
                {!isSettingUp && (
                    <button
                        onClick={triggerSetup}
                        className={cn(
                            "w-full py-2.5 px-4 rounded-xl text-sm font-bold uppercase tracking-wider",
                            "flex items-center justify-center gap-2 transition-all shadow-sm",
                            "hover:translate-y-[-1px] active:translate-y-0",
                            setupError
                                ? "bg-destructive/10 text-destructive border border-destructive/30 hover:bg-destructive/20"
                                : "bg-primary text-primary-foreground hover:opacity-90"
                        )}
                    >
                        <Zap className="w-4 h-4" />
                        {setupError ? "Retry Setup" : "Set Up Now"}
                    </button>
                )}
            </div>
        </div>
    );
}
