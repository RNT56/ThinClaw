import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    MessageSquare,
    Smartphone,
    RefreshCw,
    Shield,
    Send
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';
import { listen } from '@tauri-apps/api/event';

interface ChannelCardProps {
    id: 'whatsapp' | 'telegram' | 'slack';
    name: string;
    description: string;
    icon: any;
    status: 'connected' | 'disconnected' | 'authenticating' | 'error';
    onAction: () => void;
    actionLabel: string;
    details?: string;
}

function ChannelCard({ name, description, icon: Icon, status, onAction, actionLabel, details }: ChannelCardProps) {
    const statusConfig = {
        connected: { color: 'text-green-500 bg-green-500/10 border-green-500/20', label: 'Connected' },
        disconnected: { color: 'text-muted-foreground bg-white/5 border-white/10', label: 'Disconnected' },
        authenticating: { color: 'text-blue-500 bg-blue-500/10 border-blue-500/20', label: 'Authenticating' },
        error: { color: 'text-red-500 bg-red-500/10 border-red-500/20', label: 'Error' }
    };

    const config = statusConfig[status];

    return (
        <div className="p-6 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-white/10 flex flex-col h-full">
            <div className="flex items-start justify-between mb-4">
                <div className="p-2.5 rounded-xl bg-primary/10 border border-primary/20">
                    <Icon className="w-6 h-6 text-primary" />
                </div>
                <div className={cn("px-2.5 py-1 rounded-full text-[10px] font-bold uppercase tracking-wider border", config.color)}>
                    {config.label}
                </div>
            </div>
            <div className="flex-1">
                <h3 className="text-lg font-semibold">{name}</h3>
                <p className="text-sm text-muted-foreground mt-1 leading-relaxed">{description}</p>
                {details && <p className="text-xs text-muted-foreground/60 mt-3 font-mono">{details}</p>}
            </div>
            <button
                onClick={onAction}
                disabled={status === 'authenticating'}
                className={cn(
                    "mt-6 w-full py-2.5 rounded-xl text-sm font-medium transition-all flex items-center justify-center gap-2",
                    status === 'connected'
                        ? "bg-white/5 hover:bg-white/10 text-foreground border border-white/10"
                        : "bg-primary text-primary-foreground hover:opacity-90 shadow-lg shadow-primary/20"
                )}
            >
                {status === 'authenticating' && <RefreshCw className="w-4 h-4 animate-spin" />}
                {actionLabel}
            </button>
        </div>
    );
}

export function OpenClawChannels() {
    const [qrCode, setQrCode] = useState<string | null>(null);
    const [waStatus, setWaStatus] = useState<'connected' | 'disconnected' | 'authenticating' | 'error'>('disconnected');
    const [tgStatus, setTgStatus] = useState<'connected' | 'disconnected' | 'authenticating' | 'error'>('disconnected');
    const [slackStatus, setSlackStatus] = useState<'connected' | 'disconnected' | 'authenticating' | 'error'>('disconnected');
    const [isLoading, setIsLoading] = useState(true);

    useEffect(() => {
        const fetchData = async () => {
            try {
                const status = await openclaw.getOpenClawStatus();
                setSlackStatus(status.slack_enabled ? 'connected' : 'disconnected');
                setTgStatus(status.telegram_enabled ? 'connected' : 'disconnected');
                // WhatsApp status is dynamic and comes via events, but we can assume disconnected if not in middle of login
            } catch (e) {
                console.error('Failed to fetch channel status:', e);
            } finally {
                setIsLoading(false);
            }
        };

        fetchData();

        // Listen for login events (QR codes, etc)
        const unlisten = listen('openclaw-event', (event: any) => {
            const payload = event.payload;
            if (payload.kind === 'WebLogin') {
                if (payload.provider === 'whatsapp') {
                    if (payload.qr_code) {
                        setQrCode(payload.qr_code);
                        setWaStatus('authenticating');
                    }
                    if (payload.status === 'connected') {
                        setWaStatus('connected');
                        setQrCode(null);
                        toast.success('WhatsApp connected successfully');
                    }
                    if (payload.status === 'error') {
                        setWaStatus('error');
                        toast.error('WhatsApp connection failed');
                    }
                }
            }
        });

        return () => {
            unlisten.then(fn => fn());
        };
    }, []);

    const handleWhatsappLogin = async () => {
        try {
            setWaStatus('authenticating');
            await openclaw.loginOpenClawWhatsapp();
            toast.info('Requesting WhatsApp QR code...');
        } catch (e) {
            setWaStatus('error');
            toast.error('Failed to initiate WhatsApp login', { description: String(e) });
        }
    };

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center p-8">
                <RefreshCw className="w-8 h-8 text-primary animate-spin" />
            </div>
        );
    }

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 p-8 space-y-8 max-w-6xl mx-auto"
        >
            <div>
                <h1 className="text-3xl font-bold tracking-tight">Channel Handshakes</h1>
                <p className="text-muted-foreground mt-1">Connect your OpenClaw node to external messaging networks.</p>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                <ChannelCard
                    id="whatsapp"
                    name="WhatsApp"
                    description="Bridge your agent to WhatsApp. Supports group chats, image processing, and voice notes."
                    icon={Smartphone}
                    status={waStatus}
                    onAction={handleWhatsappLogin}
                    actionLabel={waStatus === 'connected' ? 'Re-authenticate' : 'Connect Device'}
                />
                <ChannelCard
                    id="telegram"
                    name="Telegram"
                    description="Full integration with Telegram Bot API. Ideal for low-latency notifications and commands."
                    icon={Send}
                    status={tgStatus}
                    onAction={() => toast.info('Configure Telegram in System Control')}
                    actionLabel={tgStatus === 'connected' ? 'Manage Settings' : 'Initialize Bot'}
                />
                <ChannelCard
                    id="slack"
                    name="Slack"
                    description="Enterprise communication bridge via Socket Mode. Perfect for technical teams and workspace automations."
                    icon={MessageSquare}
                    status={slackStatus}
                    onAction={() => toast.info('Configure Slack in System Control')}
                    actionLabel={slackStatus === 'connected' ? 'Manage App' : 'Setup Socket'}
                />
            </div>

            {/* QR Code Modal for WhatsApp */}
            <AnimatePresence>
                {qrCode && (
                    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4">
                        <motion.div
                            initial={{ scale: 0.9, opacity: 0 }}
                            animate={{ scale: 1, opacity: 1 }}
                            exit={{ scale: 0.9, opacity: 0 }}
                            className="bg-card border border-white/10 rounded-3xl p-8 max-w-sm w-full text-center shadow-2xl shadow-black/50"
                        >
                            <h2 className="text-2xl font-bold mb-2">Scan QR Code</h2>
                            <p className="text-sm text-muted-foreground mb-6">Open WhatsApp on your phone and scan this code to link your device.</p>

                            <div className="bg-white p-4 rounded-2xl mx-auto w-fit mb-6 shadow-inner">
                                <img
                                    src={`https://api.qrserver.com/v1/create-qr-code/?size=250x250&data=${encodeURIComponent(qrCode)}`}
                                    alt="WhatsApp QR Code"
                                    className="w-48 h-48 block"
                                />
                            </div>

                            <div className="flex flex-col gap-3">
                                <div className="flex items-center justify-center gap-2 text-blue-400 animate-pulse text-sm font-medium">
                                    <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                                    Waiting for scan...
                                </div>
                                <button
                                    onClick={() => setQrCode(null)}
                                    className="text-xs text-muted-foreground hover:text-foreground transition-colors mt-2"
                                >
                                    Cancel Authentication
                                </button>
                            </div>
                        </motion.div>
                    </div>
                )}
            </AnimatePresence>

            {/* Security Notice */}
            <div className="p-6 rounded-2xl border bg-blue-500/5 border-blue-500/20 flex gap-4">
                <div className="p-2 bg-blue-500/10 rounded-xl h-fit">
                    <Shield className="w-5 h-5 text-blue-500" />
                </div>
                <div>
                    <h4 className="text-sm font-semibold text-blue-500 uppercase tracking-wider">End-to-End Encryption</h4>
                    <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                        Communications with external channels are proxied through the OpenClaw Gateway.
                        Private keys and session tokens never leave your local node storage.
                    </p>
                </div>
            </div>
        </motion.div>
    );
}
