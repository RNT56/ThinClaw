/**
 * useEngineSetup — reusable hook for first-launch engine bootstrap.
 *
 * Checks `get_engine_setup_status` on mount, listens for
 * `engine_setup_progress` events, and exposes a `triggerSetup()` callback.
 *
 * Used by both OnboardingWizard and EngineSetupBanner.
 */
import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface EngineSetupStatus {
    needs_setup: boolean;
    setup_in_progress: boolean;
    message: string;
}

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
    useEffect(() => {
        invoke<EngineSetupStatus>("get_engine_setup_status")
            .then(setStatus)
            .catch((err) => console.warn("Failed to check engine setup:", err));
    }, []);

    // Listen for setup progress events
    useEffect(() => {
        const unlisten = listen<SetupProgress>("engine_setup_progress", (event) => {
            const { stage, message } = event.payload;
            setSetupStage(stage);
            setSetupMessage(message);

            if (stage === "complete") {
                setIsSettingUp(false);
                setSetupComplete(true);
            } else if (stage === "error") {
                setIsSettingUp(false);
                setSetupError(message);
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, []);

    const triggerSetup = useCallback(async () => {
        setIsSettingUp(true);
        setSetupError(null);
        setSetupStage("creating_venv");
        setSetupMessage("Starting setup...");

        try {
            await invoke("setup_engine");
            // Events will handle state transitions
        } catch (err: any) {
            const msg = typeof err === "string" ? err : "Setup failed";
            setIsSettingUp(false);
            setSetupError(msg);
        }
    }, []);

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
