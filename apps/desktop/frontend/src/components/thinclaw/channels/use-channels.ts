import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import * as thinclaw from '../../../lib/thinclaw';
import { useThinClawStatusSnapshot } from '../ThinClawModeBadge';
import { STREAM_MODE_LABELS } from './catalog';

export function useChannels() {
    const [channels, setChannels] = useState<thinclaw.ChannelInfo[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [expandedChannel, setExpandedChannel] = useState<string | null>(null);
    const [settingsMap, setSettingsMap] = useState<Record<string, any>>({});
    const { status: runtimeStatus, isRemote } = useThinClawStatusSnapshot(15000);
    const [appleAllowFrom, setAppleAllowFrom] = useState('');
    const [applePollInterval, setApplePollInterval] = useState('10');
    const [gmailAllowedSenders, setGmailAllowedSenders] = useState('');
    const [gmailConnecting, setGmailConnecting] = useState(false);
    const [gmailConnected, setGmailConnected] = useState(false);
    const [gmailLabelFilter, setGmailLabelFilter] = useState('');
    const [gmailStatus, setGmailStatus] = useState<thinclaw.GmailStatusResponse | null>(null);

    const fetchChannels = useCallback(async () => {
        try {
            const response = await thinclaw.getThinClawChannelsList();
            setChannels(response.channels || []);
        } catch (error) {
            console.error('Failed to fetch channels:', error);
            try {
                const status = await thinclaw.getThinClawStatus();
                setChannels([
                    {
                        id: 'slack',
                        name: 'Slack',
                        type: 'wasm',
                        enabled: status.slack_enabled,
                        stream_mode: ''
                    },
                    {
                        id: 'telegram',
                        name: 'Telegram',
                        type: 'wasm',
                        enabled: status.telegram_enabled,
                        stream_mode: ''
                    },
                    {
                        id: 'discord',
                        name: 'Discord',
                        type: 'native',
                        enabled: false,
                        stream_mode: ''
                    },
                    {
                        id: 'webhook',
                        name: 'HTTP Webhook',
                        type: 'builtin',
                        enabled: true,
                        stream_mode: ''
                    }
                ]);
            } catch {
                setChannels([]);
            }
        } finally {
            setIsLoading(false);
        }
    }, []);

    const fetchSettings = useCallback(async () => {
        try {
            const response = await thinclaw.listSettings();
            setSettingsMap(Object.fromEntries((response.settings || []).map((item) => [item.key, item.value])));
        } catch (error) {
            console.error('Failed to fetch channel settings:', error);
        }
    }, []);

    useEffect(() => {
        fetchChannels();
        fetchSettings();
    }, [fetchChannels, fetchSettings]);

    useEffect(() => {
        thinclaw
            .getGmailStatus()
            .then((status) => {
                setGmailStatus(status);
                setGmailConnected(status.oauth_configured);
                setGmailAllowedSenders(status.allowed_senders.join(', '));
                if (status.label_filters.length > 0) setGmailLabelFilter(status.label_filters.join(', '));
            })
            .catch(() => undefined);
    }, []);

    useEffect(() => {
        const allow = settingsMap['channels.apple_mail_allow_from'];
        setAppleAllowFrom(typeof allow === 'string' ? allow : '');
        const poll = settingsMap['channels.apple_mail_poll_interval'];
        setApplePollInterval(poll == null ? '10' : String(poll));
        const gmailSenders = settingsMap['channels.gmail_allowed_senders'];
        if (typeof gmailSenders === 'string' && gmailSenders.length > 0) setGmailAllowedSenders(gmailSenders);
    }, [settingsMap]);

    const settingBool = useCallback(
        (key: string, fallback = false) => {
            const value = settingsMap[key];
            if (typeof value === 'boolean') return value;
            if (typeof value === 'string') return value === 'true';
            return fallback;
        },
        [settingsMap]
    );

    const saveSetting = useCallback(
        async (key: string, value: any, label: string) => {
            try {
                await thinclaw.setSetting(key, value);
                setSettingsMap((previous) => ({ ...previous, [key]: value }));
                toast.success(`${label} updated`);
                fetchChannels();
            } catch (error) {
                toast.error(`Failed to update ${label}: ${String(error)}`);
            }
        },
        [fetchChannels]
    );

    const handleGmailConnect = useCallback(async () => {
        setGmailConnecting(true);
        try {
            const result = await thinclaw.startGmailOAuth();
            if (!result.success) {
                toast.error(result.error ?? 'Gmail connection failed');
                return;
            }
            setGmailConnected(true);
            const status = await thinclaw.getGmailStatus().catch(() => null);
            if (status) setGmailStatus(status);
            toast.success('Gmail connected successfully!');
        } catch (error) {
            toast.error(`Gmail sign-in failed: ${String(error)}`);
        } finally {
            setGmailConnecting(false);
        }
    }, []);

    const handleStreamModeChange = useCallback(async (channelId: string, mode: string) => {
        try {
            await thinclaw.setSetting(`channels.${channelId}_stream_mode`, mode);
            setChannels((previous) =>
                previous.map((channel) => (channel.id === channelId ? { ...channel, stream_mode: mode } : channel))
            );
            toast.success(`${channelId} stream mode set to ${STREAM_MODE_LABELS[mode] || mode}`);
        } catch (error) {
            toast.error(`Failed to update stream mode: ${error}`);
        }
    }, []);

    return {
        channels,
        isLoading,
        setIsLoading,
        expandedChannel,
        setExpandedChannel,
        runtimeStatus,
        isRemote,
        appleAllowFrom,
        setAppleAllowFrom,
        applePollInterval,
        setApplePollInterval,
        gmailAllowedSenders,
        setGmailAllowedSenders,
        gmailConnecting,
        gmailConnected,
        gmailLabelFilter,
        setGmailLabelFilter,
        gmailStatus,
        fetchChannels,
        fetchSettings,
        settingBool,
        saveSetting,
        handleGmailConnect,
        handleStreamModeChange
    };
}
