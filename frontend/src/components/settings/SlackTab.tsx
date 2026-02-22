import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import { MessageSquare, Key, CheckCircle, Copy, Info, AlertTriangle } from 'lucide-react';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import * as openclaw from '../../lib/openclaw';

interface SlackTabProps {
    className?: string;
}

// Slack App Manifest for easy setup
const SLACK_MANIFEST = {
    "display_information": {
        "name": "Scrappy Inference (OpenClaw)",
        "description": "Local-first OpenClaw mode inside Scrappy Inference",
        "background_color": "#111111"
    },
    "features": {
        "bot_user": {
            "display_name": "ScrappyBot",
            "always_online": false
        },
        "app_home": {
            "home_tab_enabled": true,
            "messages_tab_enabled": true,
            "messages_tab_read_only_enabled": false
        },
        "slash_commands": [
            {
                "command": "/clawd",
                "description": "Send a message to Scrappy OpenClaw",
                "should_escape": false
            }
        ]
    },
    "oauth_config": {
        "scopes": {
            "bot": [
                "app_mentions:read",
                "channels:history",
                "channels:read",
                "chat:write",
                "commands",
                "im:history",
                "im:read",
                "im:write",
                "reactions:read",
                "users:read"
            ]
        }
    },
    "settings": {
        "socket_mode_enabled": true,
        "event_subscriptions": {
            "bot_events": [
                "app_mention",
                "message.im"
            ]
        }
    }
};

export function SlackTab({ className }: SlackTabProps) {
    const [enabled, setEnabled] = useState(false);
    const [appToken, setAppToken] = useState('');
    const [botToken, setBotToken] = useState('');
    const [isLoading, setIsLoading] = useState(false);
    const [isSaved, setIsSaved] = useState(false);

    // Load initial state from backend
    useEffect(() => {
        openclaw.getOpenClawStatus().then(status => {
            setEnabled(status.slack_enabled);
        }).catch(console.error);
    }, []);

    const copyManifest = () => {
        navigator.clipboard.writeText(JSON.stringify(SLACK_MANIFEST, null, 2));
        toast.success('Manifest copied to clipboard');
    };

    const handleSave = async () => {
        setIsLoading(true);

        try {
            await openclaw.saveSlackConfig({
                enabled,
                bot_token: botToken || null,
                app_token: appToken || null,
            });
            setIsSaved(true);
            toast.success('Slack configuration saved');
            setTimeout(() => setIsSaved(false), 2000);
        } catch (e) {
            console.error('Failed to save Slack config:', e);
            toast.error('Failed to save', { description: String(e) });
        } finally {
            setIsLoading(false);
        }
    };

    const tokensValid = appToken.startsWith('xapp-') && botToken.startsWith('xoxb-');

    return (
        <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            className={cn("space-y-6", className)}
        >
            {/* Header */}
            <div className="flex items-center gap-3">
                <div className="w-10 h-10 rounded-lg bg-[#4A154B]/20 flex items-center justify-center">
                    <MessageSquare className="w-5 h-5 text-[#4A154B]" />
                </div>
                <div>
                    <h2 className="text-lg font-semibold">Slack Integration</h2>
                    <p className="text-sm text-muted-foreground">Connect OpenClaw to your Slack workspace</p>
                </div>
            </div>

            {/* Enable Toggle */}
            <div className="flex items-center justify-between p-4 rounded-lg bg-card border border-border">
                <div>
                    <p className="font-medium">Enable Slack</p>
                    <p className="text-sm text-muted-foreground">Allow OpenClaw to respond in Slack DMs</p>
                </div>
                <button
                    onClick={() => setEnabled(!enabled)}
                    className={cn(
                        "w-12 h-6 rounded-full transition-colors relative",
                        enabled ? "bg-slate-700 dark:bg-slate-300" : "bg-muted"
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
                                    <li>Create a Slack app at <a href="https://api.slack.com/apps" target="_blank" rel="noopener noreferrer" className="underline">api.slack.com/apps</a></li>
                                    <li>Use the manifest below (or configure manually)</li>
                                    <li>Enable Socket Mode and create an App-Level Token</li>
                                    <li>Install the app to your workspace</li>
                                    <li>Copy both tokens below</li>
                                </ol>
                            </div>
                        </div>
                    </div>

                    {/* Manifest */}
                    <div className="space-y-2">
                        <div className="flex items-center justify-between">
                            <label className="text-sm font-medium">App Manifest</label>
                            <button
                                onClick={copyManifest}
                                className="flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors"
                            >
                                <Copy className="w-3 h-3" />
                                Copy Manifest
                            </button>
                        </div>
                        <div className="p-3 rounded-lg bg-muted/50 border border-border text-xs font-mono max-h-32 overflow-y-auto">
                            <pre className="text-muted-foreground">{JSON.stringify(SLACK_MANIFEST, null, 2).slice(0, 200)}...</pre>
                        </div>
                    </div>

                    {/* Token Inputs */}
                    <div className="space-y-4">
                        <div className="space-y-2">
                            <label className="text-sm font-medium flex items-center gap-2">
                                <Key className="w-4 h-4" />
                                App Token (xapp-...)
                            </label>
                            <input
                                type="password"
                                value={appToken}
                                onChange={(e) => setAppToken(e.target.value)}
                                placeholder="xapp-1-..."
                                className="w-full px-3 py-2 rounded-lg bg-muted border border-border focus:border-primary focus:ring-1 focus:ring-primary outline-none transition-colors"
                            />
                            <p className="text-xs text-muted-foreground">Create under Basic Information → App-Level Tokens with `connections:write` scope</p>
                        </div>

                        <div className="space-y-2">
                            <label className="text-sm font-medium flex items-center gap-2">
                                <Key className="w-4 h-4" />
                                Bot Token (xoxb-...)
                            </label>
                            <input
                                type="password"
                                value={botToken}
                                onChange={(e) => setBotToken(e.target.value)}
                                placeholder="xoxb-..."
                                className="w-full px-3 py-2 rounded-lg bg-muted border border-border focus:border-primary focus:ring-1 focus:ring-primary outline-none transition-colors"
                            />
                            <p className="text-xs text-muted-foreground">Found under OAuth & Permissions after installing</p>
                        </div>
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
                                : tokensValid
                                    ? "bg-[#4A154B] text-white hover:bg-[#4A154B]/90 shadow-md"
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

                    {/* Validation Hint */}
                    {(appToken || botToken) && !tokensValid && (
                        <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-500/20">
                            <div className="flex items-start gap-2">
                                <AlertTriangle className="w-4 h-4 text-amber-500 mt-0.5 shrink-0" />
                                <p className="text-xs text-amber-500">
                                    Token format check: App token should start with <code className="bg-amber-500/20 px-1 rounded">xapp-</code> and Bot token with <code className="bg-amber-500/20 px-1 rounded">xoxb-</code>
                                </p>
                            </div>
                        </div>
                    )}

                    {/* Safety Note */}
                    <div className="p-3 rounded-lg bg-amber-500/10 border border-amber-500/20">
                        <div className="flex items-start gap-2">
                            <AlertTriangle className="w-4 h-4 text-amber-500 mt-0.5 shrink-0" />
                            <p className="text-xs text-amber-500">
                                <span className="font-medium">DMs only by default.</span> Channel access must be explicitly enabled and uses allowlisting for safety.
                            </p>
                        </div>
                    </div>
                </motion.div>
            )}
        </motion.div>
    );
}
