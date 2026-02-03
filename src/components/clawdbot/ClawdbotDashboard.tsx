import { useState, useEffect } from 'react';
import { motion } from 'framer-motion';
import {
    Activity,
    Shield,
    Zap,
    MessageSquare,
    Layout,
    Cpu,
    Globe,
    Lock,
    RefreshCw,
    CheckCircle2,
    XCircle,
    Database,
    Binary
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as clawdbot from '../../lib/clawdbot';
import { toast } from 'sonner';

interface StatCardProps {
    title: string;
    value: string | number;
    icon: any;
    status?: 'success' | 'warning' | 'error' | 'info';
    description?: string;
    className?: string;
}

function StatCard({ title, value, icon: Icon, status = 'info', description, className }: StatCardProps) {
    const statusColors = {
        success: 'text-green-500 bg-green-500/10 border-green-500/20',
        warning: 'text-amber-500 bg-amber-500/10 border-amber-500/20',
        error: 'text-red-500 bg-red-500/10 border-red-500/20',
        info: 'text-blue-500 bg-blue-500/10 border-blue-500/20'
    };

    return (
        <div className={cn("p-4 rounded-xl border bg-card/50 backdrop-blur-sm shadow-sm", className)}>
            <div className="flex items-start justify-between mb-2">
                <div className={cn("p-2 rounded-lg", statusColors[status])}>
                    <Icon className="w-5 h-5" />
                </div>
            </div>
            <div>
                <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider">{title}</p>
                <p className="text-2xl font-bold mt-1">{value}</p>
                {description && <p className="text-[10px] text-muted-foreground mt-1">{description}</p>}
            </div>
        </div>
    );
}

export function ClawdbotDashboard() {
    const [status, setStatus] = useState<clawdbot.ClawdbotStatus | null>(null);
    const [presence, setPresence] = useState<any>(null);
    const [isLoading, setIsLoading] = useState(true);

    const fetchData = async () => {
        try {
            const [s, p] = await Promise.all([
                clawdbot.getClawdbotStatus(),
                clawdbot.getClawdbotSystemPresence().catch(() => null)
            ]);
            setStatus(s);
            setPresence(p);
        } catch (e) {
            console.error('Failed to fetch dashboard data:', e);
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 5000);
        return () => clearInterval(interval);
    }, []);

    if (isLoading && !status) {
        return (
            <div className="flex-1 flex items-center justify-center p-8">
                <div className="flex flex-col items-center gap-4">
                    <RefreshCw className="w-8 h-8 text-primary animate-spin" />
                    <p className="text-sm text-muted-foreground">Initializing Dashboard...</p>
                </div>
            </div>
        );
    }

    const instancesCount = Array.isArray(presence?.instances) ? presence.instances.length : 0;
    const nodesCount = Array.isArray(presence?.nodes) ? presence.nodes.length : 0;

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 p-8 space-y-8 max-w-6xl mx-auto"
        >
            <div className="flex items-center justify-between">
                <div>
                    <h1 className="text-3xl font-bold tracking-tight">System Overview</h1>
                    <p className="text-muted-foreground mt-1">Real-time health and status of your OpenClaw node.</p>
                </div>
                <button
                    onClick={() => {
                        setIsLoading(true);
                        fetchData();
                        toast.success('Dashboard data refreshed');
                    }}
                    className="p-2 mr-1 rounded-lg hover:bg-white/5 transition-colors border border-white/10"
                >
                    <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                </button>
            </div>

            {/* Top Level Stats */}
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
                <StatCard
                    title="Gateway Status"
                    value={status?.gateway_running ? 'Running' : 'Offline'}
                    icon={Shield}
                    status={status?.gateway_running ? 'success' : 'error'}
                    description={status?.gateway_mode === 'local' ? 'Local Sidecar' : 'Remote Gateway'}
                />
                <StatCard
                    title="WS Connection"
                    value={status?.ws_connected ? 'Active' : 'Disconnected'}
                    icon={Zap}
                    status={status?.ws_connected ? 'success' : 'error'}
                    description={status?.ws_connected ? 'Linked to Control Plane' : 'Link interrupted'}
                />
                <StatCard
                    title="Active Instances"
                    value={instancesCount}
                    icon={Layout}
                    status={instancesCount > 0 ? 'success' : 'info'}
                    description="Connected UI clients"
                />
                <StatCard
                    title="Connected Nodes"
                    value={nodesCount}
                    icon={Cpu}
                    status={nodesCount > 0 ? 'success' : 'info'}
                    description="Managed edge devices"
                />
            </div>

            <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
                {/* Identity & Security */}
                <div className="col-span-1 lg:col-span-2 space-y-6">
                    <div className="p-6 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-white/10">
                        <div className="flex items-center gap-3 mb-6">
                            <div className="p-2 bg-primary/10 rounded-lg">
                                <Activity className="w-5 h-5 text-primary" />
                            </div>
                            <h2 className="text-lg font-semibold">Node Identity</h2>
                        </div>

                        <div className="space-y-4">
                            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                                <div className="space-y-1.5">
                                    <label className="text-[10px] uppercase font-bold text-muted-foreground tracking-wider">Device Fingerprint</label>
                                    <div className="flex items-center gap-2 bg-black/20 rounded-lg px-3 py-2 border border-white/5 font-mono text-xs">
                                        <Lock className="w-3.5 h-3.5 text-muted-foreground" />
                                        <span className="truncate">{status?.device_id}</span>
                                    </div>
                                </div>
                                <div className="space-y-1.5">
                                    <label className="text-[10px] uppercase font-bold text-muted-foreground tracking-wider">Auth Token Hash</label>
                                    <div className="flex items-center gap-2 bg-black/20 rounded-lg px-3 py-2 border border-white/5 font-mono text-xs text-muted-foreground">
                                        <Shield className="w-3.5 h-3.5" />
                                        <span>••••••••••••••••</span>
                                    </div>
                                </div>
                            </div>

                            <div className="p-4 rounded-xl bg-primary/5 border border-primary/10 flex items-start gap-3">
                                <Globe className="w-4 h-4 text-primary mt-0.5" />
                                <div>
                                    <p className="text-xs font-medium text-primary uppercase">Binding Configuration</p>
                                    <p className="text-xs text-muted-foreground mt-1">
                                        Currently listening on <span className="text-foreground font-mono">127.0.0.1:{status?.port}</span>.
                                        {status?.gateway_mode === 'remote' ? ` Connecting to ${status?.remote_url}.` : ' Internal loopback mode enabled.'}
                                    </p>
                                </div>
                            </div>
                        </div>
                    </div>

                    {/* Channel Integration Status */}
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                        <div className="p-5 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-white/10">
                            <div className="flex items-center justify-between mb-4">
                                <div className="flex items-center gap-2">
                                    <MessageSquare className="w-4 h-4 text-primary" />
                                    <h3 className="text-sm font-semibold">Slack Connect</h3>
                                </div>
                                {status?.slack_enabled ? (
                                    <CheckCircle2 className="w-4 h-4 text-green-500" />
                                ) : (
                                    <XCircle className="w-4 h-4 text-muted-foreground" />
                                )}
                            </div>
                            <p className="text-xs text-muted-foreground">
                                {status?.slack_enabled ? 'Real-time bidirectional message routing active.' : 'Slack integration is currently disabled.'}
                            </p>
                        </div>
                        <div className="p-5 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-white/10">
                            <div className="flex items-center justify-between mb-4">
                                <div className="flex items-center gap-2">
                                    <MessageSquare className="w-4 h-4 text-primary" />
                                    <h3 className="text-sm font-semibold">Telegram Hook</h3>
                                </div>
                                {status?.telegram_enabled ? (
                                    <CheckCircle2 className="w-4 h-4 text-green-500" />
                                ) : (
                                    <XCircle className="w-4 h-4 text-muted-foreground" />
                                )}
                            </div>
                            <p className="text-xs text-muted-foreground">
                                {status?.telegram_enabled ? 'Webhook listener connected and authorized.' : 'Telegram integration is currently disabled.'}
                            </p>
                        </div>
                    </div>
                </div>

                {/* System Specs & Files */}
                <div className="space-y-6">
                    <div className="p-6 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-white/10">
                        <h3 className="text-sm font-semibold mb-4 flex items-center gap-2">
                            <Database className="w-4 h-4 text-primary" />
                            Storage Paths
                        </h3>
                        <div className="space-y-3">
                            <div>
                                <p className="text-[10px] uppercase font-bold text-muted-foreground mb-1 tracking-wider">State Directory</p>
                                <div className="p-2 rounded bg-black/20 border border-white/5 text-[10px] font-mono text-muted-foreground break-all">
                                    {status?.state_dir}
                                </div>
                            </div>
                        </div>
                    </div>

                    <div className="p-6 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-white/10">
                        <h3 className="text-sm font-semibold mb-4 flex items-center gap-2">
                            <Binary className="w-4 h-4 text-primary" />
                            Software Version
                        </h3>
                        <div className="flex items-center justify-between">
                            <span className="text-xs text-muted-foreground">OpenClaw Core</span>
                            <span className="text-[10px] px-2 py-0.5 rounded-full bg-primary/10 text-primary font-mono">0.4.2-stable</span>
                        </div>
                        <div className="flex items-center justify-between mt-2">
                            <span className="text-xs text-muted-foreground">UI Version</span>
                            <span className="text-[10px] px-2 py-0.5 rounded-full bg-white/10 text-muted-foreground font-mono">0.1.0-alpha</span>
                        </div>
                    </div>
                </div>
            </div>
        </motion.div>
    );
}
