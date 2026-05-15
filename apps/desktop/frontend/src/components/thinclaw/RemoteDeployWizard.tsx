import React, { useState, useRef, useEffect } from 'react';
import { commands } from '../../lib/bindings';
import { listen } from '@tauri-apps/api/event';
import { Server, CheckCircle, AlertCircle, Loader2, Zap, Copy } from 'lucide-react';
import * as thinclaw from '../../lib/thinclaw';
import { toast } from 'sonner';

interface RemoteDeployWizardProps {
    isOpen: boolean;
    onCheckStatus?: () => void;
    onClose: () => void;
}

interface DeployResult {
    status: 'success' | 'timeout' | 'failed';
    url: string;
    token: string;
    message?: string;
}

/** Normalise a user-typed URL/IP into a clean http://host:port URL */
function normaliseHttpUrl(raw: string): string {
    let url = raw.trim();

    // Strip ws(s):// prefixes (old format — now we use HTTP)
    url = url.replace(/^wss?:\/\//, '');

    // If no protocol, add http://
    if (!/^https?:\/\//.test(url)) {
        url = `http://${url}`;
    }

    // If no port, add default 18789
    const withoutProto = url.replace(/^https?:\/\//, '');
    const hostPart = withoutProto.split('/')[0];
    if (!hostPart.includes(':')) {
        url = url.replace(hostPart, `${hostPart}:18789`);
    }

    return url;
}

export const RemoteDeployWizard: React.FC<RemoteDeployWizardProps> = ({ isOpen, onCheckStatus, onClose }) => {
    const [step, setStep] = useState<'form' | 'deploying' | 'success' | 'timeout' | 'error'>('form');
    const [ip, setIp] = useState('');
    const [user, setUser] = useState('root');
    const [logs, setLogs] = useState<string[]>([]);
    const [errorMsg, setErrorMsg] = useState('');
    const [deployMode, setDeployMode] = useState<'new' | 'existing'>('new');
    const [existingUrl, setExistingUrl] = useState('');
    const [existingToken, setExistingToken] = useState('');
    const [deployResult, setDeployResult] = useState<DeployResult | null>(null);
    const [connecting, setConnecting] = useState(false);
    const [testLoading, setTestLoading] = useState(false);
    const [tailscaleKey, setTailscaleKey] = useState('');
    const [enableSystemd, setEnableSystemd] = useState(true);

    const logEndRef = useRef<HTMLDivElement>(null);

    // Auto-scroll logs
    useEffect(() => {
        logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }, [logs]);

    const startDeploy = async () => {
        if (!ip) return;

        setStep('deploying');
        setLogs(['=== ThinClaw Remote Deploy ===', `Target: ${user}@${ip}`]);
        setErrorMsg('');
        setDeployResult(null);

        try {
            const unlistenLog = await listen<string>('deploy-log', (event) => {
                setLogs((prev) => [...prev, event.payload]);
            });

            const unlistenStatus = await listen<string>('deploy-status', (event) => {
                unlistenLog();
                unlistenStatus();

                // New structured payload
                try {
                    const result: DeployResult = JSON.parse(event.payload);
                    setDeployResult(result);
                    if (result.status === 'success') {
                        setStep('success');
                    } else if (result.status === 'timeout') {
                        setStep('timeout');
                    } else {
                        setErrorMsg(result.message || 'Deployment failed');
                        setStep('error');
                    }
                } catch {
                    // Legacy plain-text fallback
                    if (event.payload === 'success') {
                        setStep('success');
                    } else {
                        setErrorMsg(event.payload);
                        setStep('error');
                    }
                }
            });

            await commands.thinclawDeployRemote(ip, user, tailscaleKey || null, enableSystemd);

        } catch (e: any) {
            setErrorMsg(typeof e === 'string' ? e : e.message);
            setStep('error');
        }
    };

    /** Connect to the freshly deployed agent using the returned URL + token */
    const handleConnectAfterDeploy = async () => {
        if (!deployResult) return;
        setConnecting(true);

        try {
            const newProfile: thinclaw.AgentProfile = {
                id: crypto.randomUUID(),
                name: `Remote (${ip})`,
                url: deployResult.url,
                token: deployResult.token || null,
                mode: 'remote',
                auto_connect: true,
            };

            await thinclaw.addAgentProfile(newProfile);
            await commands.thinclawSaveGatewaySettings('remote', deployResult.url, deployResult.token || '');

            toast.success('Remote agent saved! Connecting...');
            onCheckStatus?.();
            onClose();
        } catch (e) {
            console.error('Failed to save profile:', e);
            toast.error('Failed to save connection profile');
        } finally {
            setConnecting(false);
        }
    };

    /** Test + connect to an existing agent */
    const handleDirectConnect = async () => {
        if (!existingUrl) return;

        const url = normaliseHttpUrl(existingUrl);

        // Test connection first
        setTestLoading(true);
        try {
            const ok = await commands.thinclawTestConnection(url, existingToken || null);
            if (!ok) {
                toast.error('Cannot connect — server unreachable or auth failed');
                setTestLoading(false);
                return;
            }
        } catch (e: any) {
            toast.error(`Connection test failed: ${typeof e === 'string' ? e : e.message}`);
            setTestLoading(false);
            return;
        }
        setTestLoading(false);

        // Save as profile and activate
        try {
            const displayHost = url.replace(/^https?:\/\//, '').split(':')[0];
            const newProfile: thinclaw.AgentProfile = {
                id: crypto.randomUUID(),
                name: `Remote (${displayHost})`,
                url,
                token: existingToken || null,
                mode: 'remote',
                auto_connect: true,
            };

            await thinclaw.addAgentProfile(newProfile);
            await commands.thinclawSaveGatewaySettings('remote', url, existingToken || '');

            toast.success('Connected to remote agent');
            onCheckStatus?.();
            onClose();
        } catch (e) {
            console.error('Failed to connect:', e);
            toast.error('Failed to connect to agent');
        }
    };

    const copyToClipboard = (text: string, label: string) => {
        navigator.clipboard.writeText(text).then(() => toast.success(`${label} copied`));
    };

    if (!isOpen) return null;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4 animate-in fade-in duration-200">
            <div className="bg-background/95 border border-border rounded-2xl shadow-2xl w-full max-w-2xl flex flex-col max-h-[90vh] overflow-hidden">
                {/* Header */}
                <div className="p-6 border-b border-border flex items-center gap-4 bg-muted/30">
                    <div className="w-12 h-12 rounded-xl bg-primary/10 flex items-center justify-center text-primary shadow-sm">
                        <Server className="w-6 h-6" />
                    </div>
                    <div>
                        <h2 className="text-xl font-bold tracking-tight text-foreground">Remote Agent Manager</h2>
                        <p className="text-sm font-medium text-muted-foreground">Deploy new agents or connect to existing deployments</p>
                    </div>
                </div>

                {/* Content */}
                <div className="flex-1 overflow-y-auto p-6 scrollbar-thin scrollbar-thumb-border scrollbar-track-transparent">

                    {step === 'form' && (
                        <div className="space-y-6">
                            {/* Tabs */}
                            <div className="flex bg-muted p-1.5 rounded-xl">
                                <button
                                    onClick={() => setDeployMode('new')}
                                    className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${deployMode === 'new' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
                                >
                                    Deploy New Agent
                                </button>
                                <button
                                    onClick={() => setDeployMode('existing')}
                                    className={`flex-1 py-2.5 text-sm font-bold rounded-lg transition-all ${deployMode === 'existing' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}`}
                                >
                                    Connect Existing
                                </button>
                            </div>

                            {deployMode === 'new' ? (
                                <div className="space-y-6 animate-in fade-in slide-in-from-left-4 duration-300">
                                    <div className="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 text-sm text-blue-600 dark:text-blue-400">
                                        <h4 className="font-bold mb-1 flex items-center gap-2"><AlertCircle size={16} /> Prerequisites</h4>
                                        <ul className="list-disc list-inside space-y-1 opacity-90 text-xs font-medium">
                                            <li>A fresh Ubuntu 22+ / Debian 12 Linux server.</li>
                                            <li>SSH access configured (key-based recommended).</li>
                                            <li>Docker, UFW Firewall & Fail2ban will be installed automatically.</li>
                                        </ul>
                                    </div>

                                    <div className="grid gap-4">
                                        <div className="space-y-2">
                                            <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Server IP Address</label>
                                            <input
                                                type="text"
                                                id="deploy-ip"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                                placeholder="e.g. 192.168.1.50 or your-server.com"
                                                value={ip}
                                                onChange={(e) => setIp(e.target.value)}
                                            />
                                        </div>

                                        <div className="space-y-2">
                                            <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">SSH User</label>
                                            <input
                                                type="text"
                                                id="deploy-user"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                                placeholder="root"
                                                value={user}
                                                onChange={(e) => setUser(e.target.value)}
                                            />
                                            <p className="text-[10px] text-muted-foreground font-medium">Usually <code>root</code> or <code>ubuntu</code> depending on your VPS provider.</p>
                                        </div>
                                    </div>

                                    {/* Security Options */}
                                    <div className="space-y-4 pt-2">
                                        <h4 className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Security Options</h4>

                                        <div className="space-y-2">
                                            <label className="text-xs font-semibold text-muted-foreground">Tailscale Auth Key <span className="text-muted-foreground/60">(optional)</span></label>
                                            <input
                                                type="text"
                                                id="deploy-tailscale"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-primary/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                                placeholder="tskey-auth-..."
                                                value={tailscaleKey}
                                                onChange={(e) => setTailscaleKey(e.target.value)}
                                            />
                                            <p className="text-[10px] text-muted-foreground font-medium">
                                                If provided, Tailscale VPN will be installed for encrypted private access. Get a key from
                                                <a href="https://login.tailscale.com/admin/settings/keys" target="_blank" rel="noopener" className="text-primary ml-1 hover:underline">Tailscale Admin</a>.
                                                Port 18789 will be restricted to Tailscale only.
                                            </p>
                                        </div>

                                        <label className="flex items-center gap-3 cursor-pointer group">
                                            <input
                                                type="checkbox"
                                                checked={enableSystemd}
                                                onChange={(e) => setEnableSystemd(e.target.checked)}
                                                className="w-4 h-4 rounded border-border text-primary focus:ring-primary/20"
                                            />
                                            <span className="text-sm font-medium text-foreground group-hover:text-primary transition-colors">
                                                Create systemd service <span className="text-muted-foreground text-xs">(auto-start on boot)</span>
                                            </span>
                                        </label>
                                    </div>
                                </div>
                            ) : (
                                <div className="space-y-6 animate-in fade-in slide-in-from-right-4 duration-300">
                                    <div className="bg-emerald-500/10 border border-emerald-500/20 rounded-xl p-4 text-sm text-emerald-600 dark:text-emerald-400">
                                        <h4 className="font-bold mb-1 flex items-center gap-2"><CheckCircle size={16} /> Direct Connection</h4>
                                        <p className="opacity-90 text-xs font-medium">Connect ThinClaw Desktop to an already running ThinClaw HTTP gateway. A connection test will be performed before saving.</p>
                                    </div>

                                    <div className="space-y-2">
                                        <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Agent URL / IP</label>
                                        <div className="relative">
                                            <input
                                                type="text"
                                                id="connect-url"
                                                className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-emerald-500/20 outline-none transition-all font-mono pl-10 placeholder:text-muted-foreground/50"
                                                placeholder="192.168.1.50 or http://your-server.com:18789"
                                                value={existingUrl}
                                                onChange={(e) => setExistingUrl(e.target.value)}
                                            />
                                            <Server className="absolute left-3 top-3.5 w-4 h-4 text-muted-foreground" />
                                        </div>
                                        <p className="text-[10px] text-muted-foreground font-medium">Port <code>18789</code> is added automatically if omitted.</p>
                                    </div>

                                    <div className="space-y-2">
                                        <label className="text-[10px] font-bold text-primary uppercase tracking-[0.2em]">Auth Token</label>
                                        <input
                                            type="password"
                                            id="connect-token"
                                            className="w-full bg-muted/50 border border-border rounded-xl px-4 py-3 text-sm focus:ring-2 focus:ring-emerald-500/20 outline-none transition-all font-mono placeholder:text-muted-foreground/50"
                                            placeholder="From GATEWAY_AUTH_TOKEN in your .env"
                                            value={existingToken}
                                            onChange={(e) => setExistingToken(e.target.value)}
                                        />
                                    </div>
                                </div>
                            )}
                        </div>
                    )}

                    {/* Log view — deploying, success, timeout, error */}
                    {(step === 'deploying' || step === 'error' || step === 'success' || step === 'timeout') && (
                        <div className="space-y-4 h-full flex flex-col">
                            <div className="flex items-center justify-between text-sm border-b border-border pb-2">
                                <span className="text-muted-foreground font-bold uppercase tracking-wider text-xs">Deployment Log</span>
                                {step === 'deploying' && (
                                    <div className="flex items-center gap-2 text-blue-500 font-bold text-xs">
                                        <Loader2 className="animate-spin w-3 h-3" /> In Progress
                                    </div>
                                )}
                                {step === 'success' && <span className="text-emerald-500 flex items-center gap-1 font-bold text-xs"><CheckCircle className="w-3 h-3" /> Deployed</span>}
                                {step === 'timeout' && <span className="text-amber-500 flex items-center gap-1 font-bold text-xs"><AlertCircle className="w-3 h-3" /> Health Check Timeout</span>}
                                {step === 'error' && <span className="text-rose-500 flex items-center gap-1 font-bold text-xs"><AlertCircle className="w-3 h-3" /> Failed</span>}
                            </div>

                            <div className="flex-1 bg-black/90 rounded-xl border border-border/50 p-4 font-mono text-[10px] leading-relaxed overflow-y-auto min-h-[200px] max-h-[300px] shadow-inner">
                                {logs.map((log, i) => (
                                    <div key={i} className={`mb-0.5 whitespace-pre-wrap ${log.includes('[stderr]') ? 'text-amber-400' : 'text-zinc-400'}`}>
                                        {log}
                                    </div>
                                ))}
                                {step === 'error' && (
                                    <div className="mt-2 text-rose-500 font-bold border-t border-rose-500/20 pt-2">Error: {errorMsg}</div>
                                )}
                                <div ref={logEndRef} />
                            </div>

                            {/* Results card — shown on success or timeout */}
                            {(step === 'success' || step === 'timeout') && deployResult && (
                                <div className={`rounded-xl border p-4 space-y-3 text-sm ${step === 'success' ? 'bg-emerald-500/10 border-emerald-500/20' : 'bg-amber-500/10 border-amber-500/20'}`}>
                                    <h4 className={`font-bold flex items-center gap-2 ${step === 'success' ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'}`}>
                                        {step === 'success' ? <CheckCircle size={16} /> : <AlertCircle size={16} />}
                                        {step === 'success' ? 'Agent Ready' : 'Agent Starting — Connect Manually'}
                                    </h4>

                                    <div className="space-y-2">
                                        <div className="flex items-center justify-between bg-black/20 rounded-lg px-3 py-2">
                                            <div>
                                                <span className="text-[10px] text-muted-foreground font-bold uppercase tracking-wider">URL</span>
                                                <p className="font-mono text-xs mt-0.5">{deployResult.url}</p>
                                            </div>
                                            <button onClick={() => copyToClipboard(deployResult.url, 'URL')} className="p-1.5 hover:bg-white/10 rounded-lg transition-colors">
                                                <Copy size={12} className="text-muted-foreground" />
                                            </button>
                                        </div>
                                        <div className="flex items-center justify-between bg-black/20 rounded-lg px-3 py-2">
                                            <div>
                                                <span className="text-[10px] text-muted-foreground font-bold uppercase tracking-wider">Auth Token</span>
                                                <p className="font-mono text-xs mt-0.5 truncate max-w-[320px]">{deployResult.token}</p>
                                            </div>
                                            <button onClick={() => copyToClipboard(deployResult.token, 'Token')} className="p-1.5 hover:bg-white/10 rounded-lg transition-colors">
                                                <Copy size={12} className="text-muted-foreground" />
                                            </button>
                                        </div>
                                    </div>
                                </div>
                            )}
                        </div>
                    )}

                </div>

                {/* Footer */}
                <div className="p-6 border-t border-border bg-muted/30 flex justify-end gap-3">
                    {step === 'form' && (
                        <>
                            <button
                                onClick={onClose}
                                className="px-5 py-2.5 rounded-xl text-muted-foreground hover:text-foreground hover:bg-muted transition-colors text-sm font-bold"
                            >
                                Cancel
                            </button>
                            <button
                                onClick={deployMode === 'new' ? startDeploy : handleDirectConnect}
                                disabled={deployMode === 'new' ? !ip : (!existingUrl || testLoading)}
                                className={`px-6 py-2.5 rounded-xl text-white text-sm font-bold shadow-lg transition-all flex items-center gap-2 ${deployMode === 'new'
                                    ? 'bg-blue-600 hover:bg-blue-500 disabled:bg-blue-600/50 shadow-blue-500/20'
                                    : 'bg-emerald-600 hover:bg-emerald-500 disabled:bg-emerald-600/50 shadow-emerald-500/20'
                                    } disabled:opacity-50 disabled:cursor-not-allowed`}
                            >
                                {testLoading ? (
                                    <><Loader2 className="w-4 h-4 animate-spin" /> Testing...</>
                                ) : deployMode === 'new' ? (
                                    <><Server className="w-4 h-4" /> Start Deployment</>
                                ) : (
                                    <><Zap className="w-4 h-4" /> Test & Connect</>
                                )}
                            </button>
                        </>
                    )}

                    {step === 'deploying' && (
                        <button
                            disabled
                            className="px-6 py-2.5 rounded-xl bg-muted text-muted-foreground text-sm font-bold cursor-wait flex items-center gap-2"
                        >
                            <Loader2 className="w-4 h-4 animate-spin" />
                            Deploying...
                        </button>
                    )}

                    {(step === 'success' || step === 'timeout') && (
                        <>
                            <button
                                onClick={onClose}
                                className="px-5 py-2.5 rounded-xl text-muted-foreground hover:text-foreground hover:bg-muted transition-colors text-sm font-bold"
                            >
                                Close
                            </button>
                            {deployResult && (
                                <button
                                    onClick={handleConnectAfterDeploy}
                                    disabled={connecting}
                                    className="px-6 py-2.5 rounded-xl bg-emerald-600 hover:bg-emerald-500 disabled:opacity-60 text-white text-sm font-bold shadow-lg shadow-emerald-500/20 transition-all flex items-center gap-2"
                                >
                                    {connecting ? (
                                        <><Loader2 className="w-4 h-4 animate-spin" /> Connecting...</>
                                    ) : (
                                        <><CheckCircle className="w-4 h-4" /> Save & Connect</>
                                    )}
                                </button>
                            )}
                        </>
                    )}

                    {step === 'error' && (
                        <button
                            onClick={() => setStep('form')}
                            className="px-6 py-2.5 rounded-xl bg-muted hover:bg-muted/80 text-foreground text-sm font-bold transition-all"
                        >
                            Try Again
                        </button>
                    )}
                </div>

            </div>
        </div>
    );
};
