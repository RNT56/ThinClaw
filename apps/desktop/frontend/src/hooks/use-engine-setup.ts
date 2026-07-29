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
    const [setupRequested, setSetupRequested] = useState(false);
    const [setupStage, setSetupStage] = useState("");
    const [setupMessage, setSetupMessage] = useState("");
    const [transientError, setTransientError] = useState<string | null>(null);

    // Check setup status on mount
    const refreshStatus = useCallback(async () => {
        directCommands.directRuntimeGetEngineSetupStatus()
            .then(unwrap)
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
                setSetupRequested(false);
                refreshStatus();
            } else if (stage === "error") {
                setSetupRequested(false);
                setTransientError(message);
                refreshStatus();
            } else {
                refreshStatus();
            }
        });

        return () => {
            unlisten.then((fn) => fn());
        };
    }, [refreshStatus]);

    const triggerSetup = useCallback(async () => {
        setSetupRequested(true);
        setTransientError(null);
        setSetupStage("creating_venv");
        setSetupMessage("Starting setup...");

        try {
            unwrap(await directCommands.directRuntimeSetupEngine());
            setSetupRequested(false);
            await refreshStatus();
        } catch (err: any) {
            const msg =
                typeof err === "string"
                    ? err
                    : err instanceof Error
                        ? err.message
                        : "Setup failed";
            setSetupRequested(false);
            setTransientError(msg);
            await refreshStatus();
        }
    }, [refreshStatus]);

    const isSettingUp = setupRequested || status?.state === "installing";
    const setupComplete = status?.state === "ready";
    const setupError = status?.error ?? transientError;
    const needsSetup = status?.state === "needs_setup" || status?.state === "broken";

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
