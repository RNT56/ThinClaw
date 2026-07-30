import React, { createContext, useContext, useState, useEffect, useCallback } from 'react';
import { commands, UserConfig, UserConfigPatch } from '../lib/bindings';
import { commandClient } from '../lib/command-client';
import { toast } from 'sonner';
import {
    requestLocalChatRuntimeRestart,
    stopLocalChatRuntime,
} from '../lib/local-runtime-start';

interface ConfigContextType {
    config: UserConfig | null;
    loading: boolean;
    updateConfig: (newConfig: UserConfig) => Promise<void>;
    refresh: () => Promise<void>;
}

const ConfigContext = createContext<ConfigContextType | undefined>(undefined);

export function ConfigProvider({ children }: { children: React.ReactNode }) {
    const [config, setConfig] = useState<UserConfig | null>(null);
    const [loading, setLoading] = useState(true);

    const fetchConfig = useCallback(async () => {
        try {
            const cfg = await commands.getUserConfig();
            setConfig(cfg);
        } catch (e) {
            console.error("Failed to load config", e);
        } finally {
            setLoading(false);
        }
    }, []);

    const updateConfig = async (newConfig: UserConfig) => {
        let attemptedLocalRuntimeStop = false;
        try {
            // Callers historically pass a full config snapshot. Send only the
            // fields they actually changed so a stale React snapshot cannot
            // overwrite a concurrent backend update.
            const patch = Object.fromEntries(
                Object.entries(newConfig).filter(([key, value]) =>
                    JSON.stringify(config?.[key as keyof UserConfig]) !== JSON.stringify(value)
                )
            ) as UserConfigPatch;

            // `selected_chat_provider` is the legacy field and
            // `chat_backend` is its replacement. Several screens still edit
            // only one of them, so normalize a provider transition into one
            // coherent patch instead of letting the two selectors disagree.
            const selectedProviderChanged =
                newConfig.selected_chat_provider !== config?.selected_chat_provider;
            const chatBackendChanged =
                newConfig.chat_backend !== config?.chat_backend;
            const requestedProvider = selectedProviderChanged
                ? newConfig.selected_chat_provider
                : chatBackendChanged
                    ? newConfig.chat_backend
                    : undefined;
            if (selectedProviderChanged || chatBackendChanged) {
                patch.selected_chat_provider = requestedProvider ?? null;
                patch.chat_backend = requestedProvider ?? 'local';
            }
            if (Object.keys(patch).length === 0) return;

            const currentProvider =
                config?.chat_backend ?? config?.selected_chat_provider ?? 'local';
            const nextProvider = requestedProvider ?? 'local';
            if (currentProvider === 'local' && nextProvider !== 'local') {
                // Free model RAM before publishing cloud state. Both runtime
                // owners are attempted by the helper, and a stop failure
                // leaves the prior provider selected.
                attemptedLocalRuntimeStop = true;
                await stopLocalChatRuntime();
            }
            await commandClient.updateUserConfig(patch);
            setConfig((current) => current ? { ...current, ...patch } : newConfig);
        } catch (e) {
            if (attemptedLocalRuntimeStop) {
                // Persistence may fail after a successful stop (or one of the
                // two owners may stop before the other reports an error).
                // Explicitly invalidate auto-start's dedupe state so the still-
                // selected local provider is restored.
                requestLocalChatRuntimeRestart();
            }
            console.error("Failed to save config", e);
            toast.error("Failed to save settings");
            throw e;
        }
    };

    useEffect(() => {
        fetchConfig();
    }, [fetchConfig]);

    return (
        <ConfigContext.Provider value={{ config, loading, updateConfig, refresh: fetchConfig }}>
            {children}
        </ConfigContext.Provider>
    );
}

export function useConfigContext() {
    const context = useContext(ConfigContext);
    if (!context) throw new Error("useConfigContext must be used within ConfigProvider");
    return context;
}
