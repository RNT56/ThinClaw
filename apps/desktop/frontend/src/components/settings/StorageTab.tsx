/**
 * A5-1: StorageTab — Cloud Storage settings page.
 * Includes: storage mode toggle, storage breakdown, provider picker,
 * config forms for all 7 providers, migration progress, recovery key.
 */
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
    Cloud, HardDrive, CheckCircle2, XCircle,
    Loader2, RefreshCw, Eye, EyeOff,
    Upload, Download, Info, Wifi, Lock, Globe, Terminal,
    Server, FolderOpen, Database, ImageIcon, FileText, Box, Cpu, ExternalLink
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import { AnimatePresence, motion } from 'framer-motion';
import {
    useCloudStatus,
    type S3ConfigInput,
    type WebDavConfigInput,
    type SftpConfigInput,
    type OAuthStartResult,
    type ConnectionTestResult,
} from '../../hooks/use-cloud-status';
import { MigrationProgressDialog } from './storage/MigrationProgressDialog';
import { RecoveryKeyPanel } from './storage/RecoveryKeyPanel';

// ── Helpers ──────────────────────────────────────────────────────────────

function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}



// ── Category icons + colors ──────────────────────────────────────────────

const CATEGORY_META: Record<string, { icon: typeof Cloud; color: string }> = {
    generated: { icon: ImageIcon, color: 'bg-purple-500' },
    documents: { icon: FileText, color: 'bg-blue-500' },
    images: { icon: ImageIcon, color: 'bg-emerald-500' },
    database: { icon: Database, color: 'bg-amber-500' },
    thinclaw_runtime_db: { icon: Cpu, color: 'bg-orange-500' },
    vectors: { icon: FolderOpen, color: 'bg-cyan-500' },
    previews: { icon: ImageIcon, color: 'bg-pink-500' },
    thinclaw: { icon: Box, color: 'bg-rose-500' },
};

// ── A5-2: StorageBreakdown — Visual bar chart ────────────────────────────

function StorageBreakdown({
    breakdown,
    totalSize,
}: {
    breakdown: { id: string; label: string; size_bytes: number }[];
    totalSize: number;
}) {
    return (
        <div className="space-y-4">
            <h3 className="text-sm font-bold uppercase tracking-wider text-muted-foreground/60">
                Storage Breakdown
            </h3>

            {/* Stacked bar */}
            <div className="h-4 rounded-full bg-muted/30 overflow-hidden flex border border-border/30">
                {breakdown.map(c => {
                    const pct = totalSize > 0 ? (c.size_bytes / totalSize) * 100 : 0;
                    if (pct < 0.5) return null;
                    const meta = CATEGORY_META[c.id];
                    return (
                        <div
                            key={c.id}
                            className={cn('h-full transition-all duration-500', meta?.color ?? 'bg-muted')}
                            style={{ width: `${pct}%` }}
                            title={`${c.label}: ${formatBytes(c.size_bytes)} (${pct.toFixed(1)}%)`}
                        />
                    );
                })}
            </div>

            {/* Legend */}
            <div className="grid grid-cols-2 gap-x-6 gap-y-1.5">
                {breakdown.map(c => {
                    const meta = CATEGORY_META[c.id];
                    const Icon = meta?.icon ?? Box;
                    return (
                        <div key={c.id} className="flex items-center gap-2 text-xs">
                            <div className={cn('w-2.5 h-2.5 rounded-full shrink-0', meta?.color ?? 'bg-muted')} />
                            <Icon className="w-3 h-3 text-muted-foreground shrink-0" />
                            <span className="text-muted-foreground truncate">{c.label}</span>
                            <span className="ml-auto font-bold text-foreground whitespace-nowrap">{formatBytes(c.size_bytes)}</span>
                        </div>
                    );
                })}
            </div>
        </div>
    );
}

// ── A5-4: S3ConfigForm ───────────────────────────────────────────────────

function S3ConfigForm({
    onTestConnection,
    testing,
}: {
    onTestConnection: (config: S3ConfigInput) => Promise<void>;
    testing: boolean;
}) {
    const [endpoint, setEndpoint] = useState('');
    const [bucket, setBucket] = useState('');
    const [region, setRegion] = useState('');
    const [accessKey, setAccessKey] = useState('');
    const [secretKey, setSecretKey] = useState('');
    const [root, setRoot] = useState('');
    const [showSecret, setShowSecret] = useState(false);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        await onTestConnection({
            endpoint: endpoint.trim() || null,
            bucket: bucket.trim(),
            region: region.trim() || null,
            access_key_id: accessKey.trim(),
            secret_access_key: secretKey.trim(),
            root: root.trim() || null,
        });
    };

    return (
        <form onSubmit={handleSubmit} className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
                <div className="col-span-2 space-y-1.5">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                        Endpoint URL <span className="text-muted-foreground/40">(leave empty for AWS)</span>
                    </label>
                    <input
                        type="text"
                        value={endpoint}
                        onChange={e => setEndpoint(e.target.value)}
                        placeholder="https://your-r2-account.r2.cloudflarestorage.com"
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all"
                    />
                </div>
                <div className="space-y-1.5">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">Bucket *</label>
                    <input type="text" value={bucket} onChange={e => setBucket(e.target.value)} placeholder="my-thinclaw-backup" required
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all" />
                </div>
                <div className="space-y-1.5">
                    <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">Region</label>
                    <input type="text" value={region} onChange={e => setRegion(e.target.value)} placeholder="auto"
                        className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all" />
                </div>
            </div>

            <InputField label="Access Key ID *" value={accessKey} onChange={setAccessKey} placeholder="AKIA..." required />
            <SecretField label="Secret Access Key *" value={secretKey} onChange={setSecretKey} show={showSecret} onToggle={() => setShowSecret(!showSecret)} placeholder="wJal..." required />
            <InputField label="Root Path" value={root} onChange={setRoot} placeholder="thinclaw-data" sublabel="(prefix inside bucket)" />

            <TestButton testing={testing} disabled={!bucket || !accessKey || !secretKey} />
        </form>
    );
}

// ── WebDAV Config Form ─────────────────────────────────────────────────

function WebDavConfigForm({
    onTestConnection,
    testing,
}: {
    onTestConnection: (config: WebDavConfigInput) => Promise<void>;
    testing: boolean;
}) {
    const [endpoint, setEndpoint] = useState('');
    const [username, setUsername] = useState('');
    const [password, setPassword] = useState('');
    const [root, setRoot] = useState('');
    const [showPassword, setShowPassword] = useState(false);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        await onTestConnection({
            endpoint: endpoint.trim(),
            username: username.trim() || null,
            password: password.trim() || null,
            root: root.trim() || null,
        });
    };

    return (
        <form onSubmit={handleSubmit} className="space-y-4">
            <InputField label="WebDAV URL *" value={endpoint} onChange={setEndpoint}
                placeholder="https://cloud.example.com/remote.php/dav/files/user/" required />
            <InputField label="Username" value={username} onChange={setUsername} placeholder="admin" />
            <SecretField label="Password" value={password} onChange={setPassword} show={showPassword}
                onToggle={() => setShowPassword(!showPassword)} placeholder="•••••" />
            <InputField label="Root Path" value={root} onChange={setRoot} placeholder="thinclaw/" sublabel="(folder on server)" />
            <TestButton testing={testing} disabled={!endpoint} />
        </form>
    );
}

// ── SFTP Config Form ───────────────────────────────────────────────────

function SftpConfigForm({
    onTestConnection,
    testing,
}: {
    onTestConnection: (config: SftpConfigInput) => Promise<void>;
    testing: boolean;
}) {
    const [endpoint, setEndpoint] = useState('');
    const [username, setUsername] = useState('');
    const [keyOrPassword, setKeyOrPassword] = useState('');
    const [root, setRoot] = useState('');
    const [showKey, setShowKey] = useState(false);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        await onTestConnection({
            endpoint: endpoint.trim(),
            username: username.trim() || null,
            key_or_password: keyOrPassword.trim() || null,
            root: root.trim() || null,
        });
    };

    return (
        <form onSubmit={handleSubmit} className="space-y-4">
            <InputField label="Host:Port *" value={endpoint} onChange={setEndpoint}
                placeholder="sftp://server.example.com:22" required />
            <InputField label="SSH Username" value={username} onChange={setUsername} placeholder="deploy" />
            <SecretField label="SSH Key Path" value={keyOrPassword} onChange={setKeyOrPassword}
                show={showKey} onToggle={() => setShowKey(!showKey)} placeholder="~/.ssh/id_rsa"
                sublabel="(path to private key)" />
            <InputField label="Remote Path" value={root} onChange={setRoot} placeholder="thinclaw/" sublabel="(directory on server)" />
            <TestButton testing={testing} disabled={!endpoint} />
        </form>
    );
}

// ── OAuth Provider Card ────────────────────────────────────────────────

function OAuthProviderCard({
    provider,
    providerName,
    testing,
    onResult,
}: {
    provider: 'gdrive' | 'dropbox' | 'onedrive';
    providerName: string;
    testing: boolean;
    onResult: (result: ConnectionTestResult) => void;
}) {
    const [loading, setLoading] = useState(false);

    const handleSignIn = async () => {
        setLoading(true);
        try {
            // Step 1: Start OAuth flow — get auth URL + code verifier
            const startResult = await invoke<OAuthStartResult>('cloud_oauth_start', { provider });

            // Step 2: Open auth URL in system browser
            try {
                await openUrl(startResult.auth_url);
            } catch {
                // Fallback: copy to clipboard
                await navigator.clipboard.writeText(startResult.auth_url);
                toast.info('Auth URL copied to clipboard. Please open it in your browser.');
            }

            // Step 3: Prompt user for authorization code
            // In production, this would use a localhost redirect listener.
            // For now, use a prompt dialog.
            const code = window.prompt(
                `After authorizing in your browser, paste the authorization code here:`
            );

            if (!code?.trim()) {
                toast.info('Sign-in cancelled');
                setLoading(false);
                return;
            }

            // Step 4: Exchange code for tokens + test connection
            const result = await invoke<ConnectionTestResult>('cloud_oauth_complete', {
                provider,
                code: code.trim(),
                codeVerifier: startResult.code_verifier,
            });

            onResult(result);

            if (result.connected) {
                toast.success(`Connected to ${result.provider_name}!`);
            } else {
                toast.error(result.error ?? 'Connection failed');
            }
        } catch (e) {
            toast.error(`${providerName} sign-in failed: ${String(e)}`);
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="space-y-4">
            <p className="text-xs text-muted-foreground leading-relaxed">
                Sign in with your {providerName} account to use it as cloud storage.
                Your data is encrypted before upload — {providerName} only sees encrypted blobs.
            </p>
            <button
                onClick={handleSignIn}
                disabled={loading || testing}
                className={cn(
                    'w-full h-11 rounded-xl font-bold text-xs uppercase tracking-wider',
                    'flex items-center justify-center gap-2 shadow-sm transition-all hover:translate-y-[-1px]',
                    'bg-primary text-primary-foreground hover:bg-primary/90',
                    (loading || testing) && 'opacity-50 cursor-not-allowed transform-none'
                )}
            >
                {loading ? (
                    <><Loader2 className="w-4 h-4 animate-spin" /> Connecting…</>
                ) : (
                    <><ExternalLink className="w-4 h-4" /> Sign in with {providerName}</>
                )}
            </button>
        </div>
    );
}

// ── iCloud Connect Card ────────────────────────────────────────────────

function ICloudConnectCard({
    testing,
    onResult,
}: {
    testing: boolean;
    onResult: (result: ConnectionTestResult) => void;
}) {
    const [loading, setLoading] = useState(false);

    const handleConnect = async () => {
        setLoading(true);
        try {
            const result = await invoke<ConnectionTestResult>('cloud_test_icloud');
            onResult(result);
            if (result.connected) {
                toast.success('Connected to iCloud Drive!');
            } else {
                toast.error(result.error ?? 'iCloud not available');
            }
        } catch (e) {
            toast.error('iCloud test failed: ' + String(e));
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="space-y-4">
            <p className="text-xs text-muted-foreground leading-relaxed">
                Uses your Mac's native iCloud Drive — no configuration needed.
                Make sure iCloud Drive is enabled in System Settings.
            </p>
            <button
                onClick={handleConnect}
                disabled={loading || testing}
                className={cn(
                    'w-full h-11 rounded-xl font-bold text-xs uppercase tracking-wider',
                    'flex items-center justify-center gap-2 shadow-sm transition-all hover:translate-y-[-1px]',
                    'bg-gradient-to-r from-sky-500 to-blue-500 text-white hover:from-sky-400 hover:to-blue-400',
                    (loading || testing) && 'opacity-50 cursor-not-allowed transform-none'
                )}
            >
                {loading ? (
                    <><Loader2 className="w-4 h-4 animate-spin" /> Checking iCloud…</>
                ) : (
                    <><Cloud className="w-4 h-4" /> Connect iCloud Drive</>
                )}
            </button>
        </div>
    );
}

// ── Shared form components ─────────────────────────────────────────────

function InputField({ label, value, onChange, placeholder, required, sublabel }: {
    label: string; value: string; onChange: (v: string) => void; placeholder: string;
    required?: boolean; sublabel?: string;
}) {
    return (
        <div className="space-y-1.5">
            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                {label} {sublabel && <span className="text-muted-foreground/40">{sublabel}</span>}
            </label>
            <input type="text" value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} required={required}
                className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all" />
        </div>
    );
}

function SecretField({ label, value, onChange, placeholder, show, onToggle, required, sublabel }: {
    label: string; value: string; onChange: (v: string) => void; placeholder: string;
    show: boolean; onToggle: () => void; required?: boolean; sublabel?: string;
}) {
    return (
        <div className="space-y-1.5">
            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                {label} {sublabel && <span className="text-muted-foreground/40">{sublabel}</span>}
            </label>
            <div className="relative">
                <input type={show ? 'text' : 'password'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} required={required}
                    className="w-full h-10 rounded-xl border border-border/50 bg-background/50 px-4 pr-12 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all" />
                <button type="button" onClick={onToggle} className="absolute right-3 top-2.5 text-muted-foreground hover:text-foreground transition-colors">
                    {show ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                </button>
            </div>
        </div>
    );
}

function TestButton({ testing, disabled }: { testing: boolean; disabled?: boolean }) {
    return (
        <button
            type="submit"
            disabled={testing || disabled}
            className={cn(
                'w-full h-11 rounded-xl bg-primary text-primary-foreground font-bold text-xs uppercase tracking-wider',
                'flex items-center justify-center gap-2 shadow-sm hover:bg-primary/90 transition-all hover:translate-y-[-1px]',
                (testing || disabled) && 'opacity-50 cursor-not-allowed transform-none'
            )}
        >
            {testing ? (
                <><Loader2 className="w-4 h-4 animate-spin" /> Testing Connection…</>
            ) : (
                <><Wifi className="w-4 h-4" /> Test Connection</>
            )}
        </button>
    );
}

// MigrationProgressDialog and RecoveryKeyPanel are in ./storage/

// ── A5-3: CloudProviderPicker ────────────────────────────────────────────

const PROVIDERS = [
    {
        id: 's3',
        name: 'S3-Compatible',
        description: 'AWS S3, Cloudflare R2, Backblaze B2, MinIO, Wasabi, DigitalOcean Spaces',
        icon: Server,
        color: 'text-blue-500',
        gradient: 'from-blue-500/20 to-sky-500/10',
        available: true,
    },
    {
        id: 'icloud',
        name: 'iCloud Drive',
        description: 'Apple iCloud — native macOS integration, zero config',
        icon: Cloud,
        color: 'text-sky-500',
        gradient: 'from-sky-500/20 to-cyan-500/10',
        available: true,
    },
    {
        id: 'gdrive',
        name: 'Google Drive',
        description: 'Google Drive via OAuth — free 15 GB tier',
        icon: Cloud,
        color: 'text-amber-500',
        gradient: 'from-amber-500/20 to-yellow-500/10',
        available: true,
    },
    {
        id: 'dropbox',
        name: 'Dropbox',
        description: 'Dropbox via OAuth — free 2 GB tier',
        icon: Cloud,
        color: 'text-blue-600',
        gradient: 'from-blue-600/20 to-indigo-500/10',
        available: true,
    },
    {
        id: 'onedrive',
        name: 'OneDrive',
        description: 'Microsoft OneDrive via OAuth — free 5 GB tier',
        icon: Cloud,
        color: 'text-indigo-500',
        gradient: 'from-indigo-500/20 to-purple-500/10',
        available: true,
    },
    {
        id: 'webdav',
        name: 'WebDAV',
        description: 'Nextcloud, ownCloud, Synology NAS, or any WebDAV server',
        icon: Globe,
        color: 'text-teal-500',
        gradient: 'from-teal-500/20 to-emerald-500/10',
        available: true,
    },
    {
        id: 'sftp',
        name: 'SFTP',
        description: 'Any Linux server, NAS, or cloud VM with SSH access',
        icon: Terminal,
        color: 'text-slate-500',
        gradient: 'from-slate-500/20 to-gray-500/10',
        available: true,
    },
];

// ── Helper: which form to render ─────────────────────────────────────────

function ProviderConfigPanel({
    providerId,
    testing,
    onS3Test,
    onWebDavTest,
    onSftpTest,
    onResult,
}: {
    providerId: string;
    testing: boolean;
    onS3Test: (config: S3ConfigInput) => Promise<void>;
    onWebDavTest: (config: WebDavConfigInput) => Promise<void>;
    onSftpTest: (config: SftpConfigInput) => Promise<void>;
    onResult: (result: ConnectionTestResult) => void;
}) {
    const meta = PROVIDERS.find(p => p.id === providerId);
    const Icon = meta?.icon ?? Cloud;

    return (
        <div className="p-6 rounded-2xl border border-border/50 bg-card/40 shadow-sm space-y-5">
            <h3 className="font-bold text-base flex items-center gap-2">
                <Icon className={cn('w-4 h-4', meta?.color ?? 'text-primary')} />
                {meta?.name ?? providerId} Connection
            </h3>

            {providerId === 's3' && <S3ConfigForm onTestConnection={onS3Test} testing={testing} />}
            {providerId === 'icloud' && <ICloudConnectCard testing={testing} onResult={onResult} />}
            {providerId === 'gdrive' && <OAuthProviderCard provider="gdrive" providerName="Google Drive" testing={testing} onResult={onResult} />}
            {providerId === 'dropbox' && <OAuthProviderCard provider="dropbox" providerName="Dropbox" testing={testing} onResult={onResult} />}
            {providerId === 'onedrive' && <OAuthProviderCard provider="onedrive" providerName="Microsoft OneDrive" testing={testing} onResult={onResult} />}
            {providerId === 'webdav' && <WebDavConfigForm onTestConnection={onWebDavTest} testing={testing} />}
            {providerId === 'sftp' && <SftpConfigForm onTestConnection={onSftpTest} testing={testing} />}
        </div>
    );
}

// ── A5-1: Main StorageTab Component ──────────────────────────────────────

export function StorageTab() {
    const {
        status, breakdown, totalSize, migrationProgress, loading, error: _error,
        isCloud, isLocal, isMigrating, refresh, refreshBreakdown, setMigrationProgress,
    } = useCloudStatus();

    const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
    const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);
    const [testing, setTesting] = useState(false);
    const [cancelling, setCancelling] = useState(false);
    const [showMigrationDialog, setShowMigrationDialog] = useState(false);

    // Auto-show migration dialog when migration starts
    useEffect(() => {
        if (isMigrating && migrationProgress) {
            setShowMigrationDialog(true);
        }
    }, [isMigrating, migrationProgress]);

    const handleS3TestConnection = async (config: S3ConfigInput) => {
        setTesting(true);
        setTestResult(null);
        try {
            const result = await invoke<ConnectionTestResult>('cloud_test_connection', { config });
            setTestResult(result);
            if (result.connected) {
                toast.success(`Connected to ${result.provider_name}!`);
            } else {
                toast.error(result.error ?? 'Connection failed');
            }
        } catch (e) {
            toast.error('Connection test failed: ' + String(e));
        } finally {
            setTesting(false);
        }
    };

    const handleWebDavTestConnection = async (config: WebDavConfigInput) => {
        setTesting(true);
        setTestResult(null);
        try {
            const result = await invoke<ConnectionTestResult>('cloud_test_webdav', { config });
            setTestResult(result);
            if (result.connected) {
                toast.success(`Connected to ${result.provider_name}!`);
            } else {
                toast.error(result.error ?? 'Connection failed');
            }
        } catch (e) {
            toast.error('WebDAV connection test failed: ' + String(e));
        } finally {
            setTesting(false);
        }
    };

    const handleSftpTestConnection = async (config: SftpConfigInput) => {
        setTesting(true);
        setTestResult(null);
        try {
            const result = await invoke<ConnectionTestResult>('cloud_test_sftp', { config });
            setTestResult(result);
            if (result.connected) {
                toast.success(`Connected to ${result.provider_name}!`);
            } else {
                toast.error(result.error ?? 'Connection failed');
            }
        } catch (e) {
            toast.error('SFTP connection test failed: ' + String(e));
        } finally {
            setTesting(false);
        }
    };

    const handleOAuthResult = (result: ConnectionTestResult) => {
        setTestResult(result);
    };

    const handleMigrateToCloud = async () => {
        try {
            setShowMigrationDialog(true);
            await invoke('cloud_migrate_to_cloud');
        } catch (e) {
            toast.error('Migration failed: ' + String(e));
        }
    };

    const handleMigrateToLocal = async () => {
        try {
            setShowMigrationDialog(true);
            await invoke('cloud_migrate_to_local');
        } catch (e) {
            toast.error('Migration failed: ' + String(e));
        }
    };

    const handleCancelMigration = async () => {
        setCancelling(true);
        try {
            await invoke('cloud_cancel_migration');
            toast.info('Migration cancellation requested');
        } catch (e) {
            toast.error('Failed to cancel: ' + String(e));
        } finally {
            setCancelling(false);
        }
    };

    const handleCloseMigrationDialog = () => {
        setShowMigrationDialog(false);
        setMigrationProgress(null);
        refresh();
        refreshBreakdown();
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center p-20">
                <Loader2 className="w-8 h-8 animate-spin text-primary/50" />
            </div>
        );
    }

    return (
        <div className="space-y-8 pb-20">

            {/* ── Current Mode Card ────────────────────────────────────── */}
            <div className="relative overflow-hidden rounded-2xl border border-border/50 bg-card/40 shadow-sm">
                <div className={cn(
                    'absolute inset-0 bg-gradient-to-br pointer-events-none opacity-40',
                    isCloud ? 'from-blue-500/20 to-sky-500/10' : 'from-emerald-500/20 to-green-500/10'
                )} />
                <div className="relative p-6 space-y-4">
                    <div className="flex items-center justify-between">
                        <div className="flex items-center gap-3">
                            <div className={cn(
                                'p-3 rounded-xl',
                                isCloud ? 'bg-blue-500/10' : 'bg-emerald-500/10'
                            )}>
                                {isCloud ? (
                                    <Cloud className="w-6 h-6 text-blue-500" />
                                ) : (
                                    <HardDrive className="w-6 h-6 text-emerald-500" />
                                )}
                            </div>
                            <div>
                                <h2 className="text-xl font-bold">
                                    {isCloud ? 'Cloud Storage' : 'Local Storage'}
                                </h2>
                                <p className="text-sm text-muted-foreground">
                                    {isCloud
                                        ? `Connected to ${status?.provider_name ?? 'cloud provider'}. Data is encrypted and synced.`
                                        : 'All data stored on this device. Private and offline-capable.'
                                    }
                                </p>
                            </div>
                        </div>

                        <div className="flex items-center gap-3">
                            {isCloud && (
                                <div className="flex items-center gap-1.5 px-3 py-1.5 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 rounded-full text-[10px] font-bold uppercase tracking-wider border border-emerald-500/20">
                                    <Lock className="w-3 h-3" />
                                    End-to-End Encrypted
                                </div>
                            )}
                            <button
                                onClick={() => { refresh(); refreshBreakdown(); }}
                                className="p-2 hover:bg-muted/50 rounded-lg transition-colors"
                                title="Refresh"
                            >
                                <RefreshCw className="w-4 h-4 text-muted-foreground" />
                            </button>
                        </div>
                    </div>

                    {/* Quick stats */}
                    <div className="flex gap-6 text-xs">
                        <div>
                            <span className="text-muted-foreground">Total Size:</span>
                            <span className="ml-1.5 font-bold text-foreground">{formatBytes(totalSize)}</span>
                        </div>
                        {status?.last_sync_at && (
                            <div>
                                <span className="text-muted-foreground">Last Sync:</span>
                                <span className="ml-1.5 font-bold text-foreground">
                                    {new Date(status.last_sync_at * 1000).toLocaleString()}
                                </span>
                            </div>
                        )}
                        {isCloud && status?.storage_available != null && (
                            <div>
                                <span className="text-muted-foreground">Cloud Usage:</span>
                                <span className="ml-1.5 font-bold text-foreground">
                                    {formatBytes(status?.storage_used ?? 0)}
                                    {status.storage_available > 0 && ` / ${formatBytes(status.storage_available)}`}
                                </span>
                            </div>
                        )}
                    </div>
                </div>
            </div>

            {/* ── Storage Breakdown ────────────────────────────────────── */}
            <div className="p-6 rounded-2xl border border-border/50 bg-card/40 shadow-sm">
                <StorageBreakdown breakdown={breakdown} totalSize={totalSize} />
            </div>

            {/* ── Cloud Provider Picker ────────────────────────────────── */}
            {isLocal && (
                <div className="space-y-4">
                    <div className="flex items-center gap-2">
                        <Cloud className="w-4 h-4 text-primary" />
                        <h3 className="text-sm font-bold uppercase tracking-wider text-muted-foreground/60">
                            Enable Cloud Storage
                        </h3>
                    </div>

                    <div className="grid grid-cols-2 gap-3">
                        {PROVIDERS.map(p => {
                            const Icon = p.icon;
                            const isSelected = selectedProvider === p.id;
                            return (
                                <button
                                    key={p.id}
                                    onClick={() => p.available && setSelectedProvider(isSelected ? null : p.id)}
                                    disabled={!p.available}
                                    className={cn(
                                        'relative overflow-hidden rounded-2xl border p-5 text-left transition-all duration-300',
                                        isSelected
                                            ? 'border-primary bg-primary/5 ring-2 ring-primary/20 shadow-md'
                                            : p.available
                                                ? 'border-border/50 bg-card/40 hover:bg-card/60 hover:border-border cursor-pointer shadow-sm'
                                                : 'border-border/30 bg-muted/20 opacity-50 cursor-not-allowed'
                                    )}
                                >
                                    <div className={cn(
                                        'absolute inset-0 bg-gradient-to-br opacity-0 transition-opacity duration-500 pointer-events-none',
                                        isSelected && 'opacity-100',
                                        p.gradient
                                    )} />
                                    <div className="relative space-y-2">
                                        <div className="flex items-center gap-2.5">
                                            <Icon className={cn('w-5 h-5', p.available ? p.color : 'text-muted-foreground')} />
                                            <span className="font-bold text-sm">{p.name}</span>
                                            {!p.available && (
                                                <span className="ml-auto text-[9px] font-bold uppercase bg-muted/50 text-muted-foreground px-2 py-0.5 rounded">
                                                    Coming Soon
                                                </span>
                                            )}
                                        </div>
                                        <p className="text-xs text-muted-foreground leading-relaxed">{p.description}</p>
                                    </div>
                                </button>
                            );
                        })}
                    </div>
                </div>
            )}

            {/* ── Provider Config Panel ─────────────────────────────────── */}
            <AnimatePresence>
                {selectedProvider && isLocal && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        transition={{ duration: 0.3 }}
                        className="overflow-hidden"
                    >
                        <ProviderConfigPanel
                            providerId={selectedProvider}
                            testing={testing}
                            onS3Test={handleS3TestConnection}
                            onWebDavTest={handleWebDavTestConnection}
                            onSftpTest={handleSftpTestConnection}
                            onResult={handleOAuthResult}
                        />

                        {/* Connection test result */}
                        <AnimatePresence>
                            {testResult && (
                                <motion.div
                                    initial={{ height: 0, opacity: 0 }}
                                    animate={{ height: 'auto', opacity: 1 }}
                                    exit={{ height: 0, opacity: 0 }}
                                    className="mt-4"
                                >
                                    <div className={cn(
                                        'p-4 rounded-xl border',
                                        testResult.connected
                                            ? 'bg-emerald-500/5 border-emerald-500/20'
                                            : 'bg-rose-500/5 border-rose-500/20'
                                    )}>
                                        <div className="flex items-center gap-2 mb-2">
                                            {testResult.connected ? (
                                                <CheckCircle2 className="w-4 h-4 text-emerald-500" />
                                            ) : (
                                                <XCircle className="w-4 h-4 text-rose-500" />
                                            )}
                                            <span className="font-bold text-sm">
                                                {testResult.connected ? `Connected to ${testResult.provider_name}` : 'Connection Failed'}
                                            </span>
                                        </div>
                                        {testResult.connected && (
                                            <p className="text-xs text-muted-foreground">
                                                Storage: {formatBytes(testResult.storage_used)} used
                                                {testResult.storage_available != null && ` / ${formatBytes(testResult.storage_available)} available`}
                                            </p>
                                        )}
                                        {testResult.error && (
                                            <p className="text-xs text-rose-600 dark:text-rose-400 mt-1">{testResult.error}</p>
                                        )}

                                        {/* Migrate button */}
                                        {testResult.connected && (
                                            <button
                                                onClick={handleMigrateToCloud}
                                                className="mt-4 w-full h-11 rounded-xl bg-gradient-to-r from-blue-500 to-sky-500 text-white font-bold text-xs uppercase tracking-wider
                                                    flex items-center justify-center gap-2 shadow-lg shadow-blue-500/20 hover:shadow-blue-500/30
                                                    hover:translate-y-[-1px] transition-all"
                                            >
                                                <Upload className="w-4 h-4" />
                                                Migrate to Cloud
                                            </button>
                                        )}
                                    </div>
                                </motion.div>
                            )}
                        </AnimatePresence>
                    </motion.div>
                )}
            </AnimatePresence>

            {/* ── Migrate Back to Local ────────────────────────────────── */}
            {isCloud && (
                <div className="p-6 rounded-2xl border border-border/50 bg-card/40 shadow-sm space-y-4">
                    <div className="flex items-center gap-3">
                        <div className="p-2.5 rounded-xl bg-emerald-500/10">
                            <HardDrive className="w-5 h-5 text-emerald-500" />
                        </div>
                        <div>
                            <h3 className="font-bold text-base">Switch to Local Storage</h3>
                            <p className="text-xs text-muted-foreground">
                                Download all data from the cloud and switch back to local-only mode.
                            </p>
                        </div>
                    </div>
                    <button
                        onClick={handleMigrateToLocal}
                        disabled={isMigrating}
                        className={cn(
                            'w-full h-11 rounded-xl border border-emerald-500/30 text-emerald-600 dark:text-emerald-400 font-bold text-xs uppercase tracking-wider',
                            'flex items-center justify-center gap-2 hover:bg-emerald-500/10 transition-all',
                            isMigrating && 'opacity-50 cursor-not-allowed'
                        )}
                    >
                        {isMigrating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Download className="w-4 h-4" />}
                        Migrate to Local
                    </button>
                </div>
            )}

            {/* ── Recovery Key Panel ───────────────────────────────────── */}
            <RecoveryKeyPanel />

            {/* ── Info Footer ─────────────────────────────────────────── */}
            <div className="p-5 rounded-2xl border border-primary/10 bg-primary/5 flex gap-4 items-start">
                <div className="p-2.5 bg-primary/10 rounded-xl shrink-0">
                    <Info className="w-5 h-5 text-primary" />
                </div>
                <div className="space-y-1.5">
                    <p className="text-sm font-bold">How Cloud Storage Works</p>
                    <p className="text-xs text-muted-foreground leading-relaxed">
                        All files are <span className="text-foreground font-medium">encrypted client-side</span> with AES-256-GCM before upload.
                        Your encryption key never leaves this device. The cloud provider only sees encrypted blobs — they cannot read your data.
                        You can migrate freely between local and cloud modes at any time. Use the recovery key to access your data on a new device.
                    </p>
                </div>
            </div>

            {/* ── Migration Progress Dialog ────────────────────────────── */}
            <AnimatePresence>
                {showMigrationDialog && migrationProgress && (
                    <MigrationProgressDialog
                        progress={migrationProgress}
                        onCancel={handleCancelMigration}
                        cancelling={cancelling}
                        onClose={handleCloseMigrationDialog}
                    />
                )}
            </AnimatePresence>
        </div>
    );
}
