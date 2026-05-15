/**
 * useEngineSetup — reusable hook for first-launch engine bootstrap.
 *
 * Checks `direct_runtime_get_engine_setup_status` on mount, listens for
 * `engine_setup_progress` events, and exposes a `triggerSetup()` callback.
 *
 * Used by both OnboardingWizard and EngineSetupBanner.
 */
import { useState, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { EngineSetupStatus } from "../lib/bindings";
import { directCommands } from "../lib/generated/direct-commands";
import { unwrap } from "../lib/utils";

interface SetupProgress {
    stage: string; // "creating_venv" | "installing" | "complete" | "error"
    message: string;
}

export interface EngineSetupState {
    status: EngineSetupStatus | null;
    isSettingUp: boolean;
    setupStage: string;
    setupMessage: string;
    setupComplete: boolean;
    setupError: string | null;
    triggerSetup: () => Promise<void>;
    /** Derived: setup is needed and hasn't completed yet */
    needsSetup: boolean;
}

export function useEngineSetup(): EngineSetupState {
    const [status, setStatus] = useState<EngineSetupStatus | null>(null);
    const [isSettingUp, setIsSettingUp] = useState(false);
    const [setupStage, setSetupStage] = useState("");
    const [setupMessage, setSetupMessage] = useState("");
    const [setupComplete, setSetupComplete] = useState(false);
    const [setupError, setSetupError] = useState<string | null>(null);

    // Check setup status on mount
    const refreshStatus = useCallback(async () => {
        directCommands.directRuntimeGetEngineSetupStatus()
            .then(setStatus)
            .catch((err) => console.warn("Failed to check engine setup:", err));
    }, []);

    useEffect(() => {
        refreshStatus();
    }, [refreshStatus]);

    // Listen for setup progress events
    useEffect(() => {
        const unlisten = listen<SetupProgress>("engine_setup_progress", (event) => {
            const { stage, message } = event.payload;
            setSetupStage(stage);
            setSetupMessage(message);

            if (stage === "complete") {
                setIsSettingUp(false);
                setSetupComplete(true);
                refreshStatus();
            } else if (stage === "error") {
                setIsSettingUp(false);
                setSetupError(message);
                refreshStatus();
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [refreshStatus]);

    const triggerSetup = useCallback(async () => {
        setIsSettingUp(true);
        setSetupError(null);
        setSetupStage("creating_venv");
        setSetupMessage("Starting setup...");

        try {
            unwrap(await directCommands.directRuntimeSetupEngine());
            setSetupComplete(true);
            setIsSettingUp(false);
            await refreshStatus();
        } catch (err: any) {
            const msg = typeof err === "string" ? err : "Setup failed";
            setIsSettingUp(false);
            setSetupError(msg);
        }
    }, [refreshStatus]);

    const needsSetup = !!(status?.needs_setup && !setupComplete);

    return {
        status,
        isSettingUp,
        setupStage,
        setupMessage,
        setupComplete,
        setupError,
        triggerSetup,
        needsSetup,
    };
}
