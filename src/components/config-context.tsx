import React, { createContext, useContext, useState, useEffect, useCallback } from 'react';
import { commands, UserConfig } from '../lib/bindings';
import { toast } from 'sonner';

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
        try {
            // Optimistic update
            setConfig(newConfig);
            await commands.updateUserConfig(newConfig);
        } catch (e) {
            console.error("Failed to save config", e);
            toast.error("Failed to save settings");
            fetchConfig(); // Revert
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
