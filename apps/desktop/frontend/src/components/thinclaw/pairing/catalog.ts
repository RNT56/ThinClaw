export interface PairingChannel {
    id: string;
    label: string;
    setup: string;
}

/** Channels whose runtime adapters actually emit and consume DM pairing codes. */
export const PAIRING_CHANNELS: readonly PairingChannel[] = [
    { id: 'telegram', label: 'Telegram', setup: 'Bot token' },
    { id: 'slack', label: 'Slack', setup: 'App credentials' },
    { id: 'discord', label: 'Discord', setup: 'Application credentials' },
    { id: 'whatsapp', label: 'WhatsApp', setup: 'Cloud API credentials' },
    { id: 'signal', label: 'Signal', setup: 'signal-cli account' },
];
