import { useState, useEffect } from 'react';
import { commands, UserConfig } from '../lib/bindings';
import { toast } from 'sonner';



export function useConfig() {
    const [config, setConfig] = useState<UserConfig | null>(null);
    const [loading, setLoading] = useState(true);

    const fetchConfig = async () => {
        try {
            const cfg = await commands.getUserConfig();
            setConfig(cfg);
        } catch (e) {
            console.error("Failed to load config", e);
            toast.error("Failed to load user config");
        } finally {
            setLoading(false);
        }
    };

    const updateConfig = async (newConfig: UserConfig) => {
        try {
            await commands.updateUserConfig(newConfig);
            setConfig(newConfig);
            toast.success("Settings saved");
        } catch (e) {
            console.error("Failed to save config", e);
            toast.error("Failed to save settings");
        }
    };



    useEffect(() => {
        fetchConfig();
    }, []);

    return {
        config,
        loading,
        updateConfig,
        refresh: fetchConfig
    };
}
