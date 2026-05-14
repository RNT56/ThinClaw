import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import { Send, Key, CheckCircle, Info, AlertTriangle, Users } from 'lucide-react';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import * as openclaw from '../../lib/openclaw';

interface TelegramTabProps {
    className?: string;
}

export function TelegramTab({ className }: TelegramTabProps) {
    const [enabled, setEnabled] = useState(false);
    const [botToken, setBotToken] = useState('');
    const [isLoading, setIsLoading] = useState(false);
    const [isSaved, setIsSaved] = useState(false);
    const [dmPolicy, setDmPolicy] = useState<'open' | 'pairing'>('pairing');
    const [groupsEnabled, setGroupsEnabled] = useState(false);

    // Load initial state from backend
    useEffect(() => {
        openclaw.getOpenClawStatus().then(status => {
            setEnabled(status.telegram_enabled);
        }).catch(console.error);
    }, []);

    const handleSave = async () => {
        setIsLoading(true);

        try {
            await openclaw.saveTelegramConfig({
                enabled,
                bot_token: botToken || null,
                dm_policy: dmPolicy,
                groups_enabled: groupsEnabled,
            });
            setIsSaved(true);
            toast.success('Telegram configuration saved');
            setTimeout(() => setIsSaved(false), 2000);
        } catch (e) {
            console.error('Failed to save Telegram config:', e);
            toast.error('Failed to save', { description: String(e) });
        } finally {
            setIsLoading(false);
        }
    };

    const tokenValid = botToken.includes(':');

    return (
        <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            className={cn("space-y-6", className)}
        >
            {/* Header */}
            <div className="flex items-center gap-3">
                <div className="w-10 h-10 rounded-lg bg-[#0088cc]/20 flex items-center justify-center">
                    <Send className="w-5 h-5 text-[#0088cc]" />
                </div>
                <div>
                    <h2 className="text-lg font-semibold">Telegram Integration</h2>
                    <p className="text-sm text-muted-foreground">Connect ThinClaw to Telegram</p>
                </div>
            </div>

            {/* Enable Toggle */}
            <div className="flex items-center justify-between p-4 rounded-lg bg-card border border-border">
                <div>
                    <p className="font-medium">Enable Telegram</p>
                    <p className="text-sm text-muted-foreground">Allow ThinClaw to respond via Telegram</p>
                </div>
                <button
                    onClick={() => setEnabled(!enabled)}
                    className={cn(
                        "w-12 h-6 rounded-full transition-colors relative",
                        enabled ? "bg-[#0088cc]" : "bg-muted"
                    )}
                >
                    <div className={cn(
                        "w-5 h-5 rounded-full bg-white absolute top-0.5 transition-transform",
                        enabled ? "translate-x-6" : "translate-x-0.5"
                    )} />
                </button>
            </div>

            {enabled && (
                <motion.div
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: 'auto' }}
                    className="space-y-4"
                >
                    {/* Setup Instructions */}
                    <div className="p-4 rounded-lg bg-blue-500/10 border border-blue-500/20">
                        <div className="flex items-start gap-2">
                            <Info className="w-4 h-4 text-blue-500 mt-0.5 shrink-0" />
                            <div className="text-sm text-blue-500">
                                <p className="font-medium mb-2">Setup Steps:</p>
                                <ol className="list-decimal list-inside space-y-1 text-blue-400">
                                    <li>Message <a href="https://t.me/BotFather" target="_blank" rel="noopener noreferrer" className="underline">@BotFather</a> on Telegram</li>
                                    <li>Send /newbot and follow the prompts</li>
                                    <li>Copy the API token provided</li>
                                    <li>Paste the token below</li>
                                </ol>
                            </div>
                        </div>
                    </div>

                    {/* Token Input */}
                    <div className="space-y-2">
                        <label className="text-sm font-medium flex items-center gap-2">
                            <Key className="w-4 h-4" />
                            Bot Token
                        </label>
                        <input
                            type="password"
                            value={botToken}
                            onChange={(e) => setBotToken(e.target.value)}
                            placeholder="123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
                            className="w-full px-3 py-2 rounded-lg bg-muted border border-border focus:border-primary focus:ring-1 focus:ring-primary outline-none transition-colors font-mono text-sm"
                        />
                        <p className="text-xs text-muted-foreground">The token from BotFather (format: ID:SECRET)</p>
                    </div>

                    {/* DM Policy */}
                    <div className="space-y-3">
                        <label className="text-sm font-medium">DM Access Policy</label>
                        <div className="grid grid-cols-2 gap-2">
                            <button
                                onClick={() => setDmPolicy('pairing')}
                                className={cn(
                                    "p-3 rounded-lg border text-left transition-all",
                                    dmPolicy === 'pairing'
                                        ? "border-[#0088cc] bg-[#0088cc]/10"
                                        : "border-border hover:border-muted-foreground"
                                )}
                            >
                                <p className="font-medium text-sm">Pairing Required</p>
                                <p className="text-xs text-muted-foreground">Users must verify via code</p>
                            </button>
                            <button
                                onClick={() => setDmPolicy('open')}
                                className={cn(
                                    "p-3 rounded-lg border text-left transition-all",
                                    dmPolicy === 'open'
                                        ? "border-[#0088cc] bg-[#0088cc]/10"
                                        : "border-border hover:border-muted-foreground"
                                )}
                            >
                                <p className="font-medium text-sm">Open</p>
                                <p className="text-xs text-muted-foreground">Anyone can DM the bot</p>
                            </button>
                        </div>
                    </div>

                    {/* Group Access */}
                    <div className="flex items-center justify-between p-4 rounded-lg bg-muted/50 border border-border">
                        <div className="flex items-center gap-3">
                            <Users className="w-4 h-4 text-muted-foreground" />
                            <div>
                                <p className="font-medium text-sm">Group Messages</p>
                                <p className="text-xs text-muted-foreground">Respond when @mentioned in groups</p>
                            </div>
                        </div>
                        <button
                            onClick={() => setGroupsEnabled(!groupsEnabled)}
                            className={cn(
                                "w-10 h-5 rounded-full transition-colors relative",
                                groupsEnabled ? "bg-[#0088cc]" : "bg-border"
                            )}
                        >
                            <div className={cn(
                                "w-4 h-4 rounded-full bg-white absolute top-0.5 transition-transform",
                                groupsEnabled ? "translate-x-5" : "translate-x-0.5"
                            )} />
                        </button>
                    </div>

                    {/* Save Button */}
                    <button
                        onClick={handleSave}
                        disabled={isLoading}
                        className={cn(
                            "w-full py-2.5 rounded-lg font-medium transition-all",
                            "flex items-center justify-center gap-2",
                            isLoading
                                ? "bg-muted text-muted-foreground cursor-wait"
                                : tokenValid
                                    ? "bg-[#0088cc] text-white hover:bg-[#0088cc]/90 shadow-md"
                                    : "bg-muted text-muted-foreground"
                        )}
                    >
                        {isLoading ? (
                            <>
                                <div className="w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin" />
                                Saving...
                            </>
                        ) : isSaved ? (
                            <>
                                <CheckCircle className="w-4 h-4" />
                                Saved
                            </>
                        ) : (
                            'Save Configuration'
                        )}
                    </button>

                    {/* Safety Note */}
                    {dmPolicy === 'open' && (
                        <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-500/20">
                            <div className="flex items-start gap-2">
                                <AlertTriangle className="w-4 h-4 text-amber-500 mt-0.5 shrink-0" />
                                <p className="text-xs text-amber-500">
                                    <span className="font-medium">Open DM policy:</span> Any Telegram user can start a conversation with your bot. Consider using pairing for better control.
                                </p>
                            </div>
                        </div>
                    )}
                </motion.div>
            )}
        </motion.div>
    );
}
