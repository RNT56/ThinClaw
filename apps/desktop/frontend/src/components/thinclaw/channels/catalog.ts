import { Globe, Hash, Mail, MessageCircle, MessageSquare, Radio, Send, Shield, Smartphone, Wifi } from 'lucide-react';

export const CHANNEL_ICONS = {
    slack: MessageSquare,
    telegram: Send,
    discord: Hash,
    signal: Shield,
    webhook: Globe,
    nostr: Radio,
    whatsapp: Smartphone,
    gmail: Mail,
    apple_mail: Mail,
    imessage: MessageCircle,
    bluebubbles: Smartphone
} as const;

export const CHANNEL_DESCRIPTIONS: Record<string, string> = {
    slack: 'Enterprise workspace bridge via Socket Mode. Supports streaming draft replies.',
    telegram: 'Full Telegram Bot API integration with forum topics, channel posts, and DM pairing.',
    discord: 'Native Rust gateway with WebSocket + REST. Streaming draft replies support.',
    signal: 'Encrypted messaging via signal-cli daemon with SSE listener.',
    webhook: 'HTTP webhook endpoint with HMAC-SHA256 signature verification. Always active.',
    nostr: 'NIP-04 encrypted direct messages on the Nostr protocol.',
    whatsapp: 'Bridge to WhatsApp via web authentication. Supports media and group chats.',
    gmail: 'Gmail integration via OAuth 2.0. Read and respond to email with label-based filtering.',
    apple_mail: 'Apple Mail integration (macOS only). Reads the local Envelope Index and sends via Mail.app.',
    imessage: 'iMessage channel (macOS only). Polls chat.db for incoming messages and responds via AppleScript.',
    bluebubbles:
        'Cross-platform iMessage bridge via BlueBubbles server. Supports media, read receipts, and group chats.'
};

export const STREAM_MODES = ['', 'edit', 'status', 'chunks'] as const;

export const STREAM_MODE_LABELS: Record<string, string> = {
    '': 'Off (full reply)',
    edit: 'Live Edit',
    status: 'Typing Indicator',
    chunks: 'Chunked'
};

export function channelIcon(channelId: string) {
    return CHANNEL_ICONS[channelId as keyof typeof CHANNEL_ICONS] ?? Wifi;
}
