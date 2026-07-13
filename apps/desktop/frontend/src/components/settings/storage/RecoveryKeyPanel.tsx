/**
 * RecoveryKeyPanel — explicit reveal/import controls for cloud data or the
 * Desktop secret envelope, plus transactional secret master-key rotation.
 */
import { useEffect, useState } from 'react';
import {
    AlertTriangle,
    Copy,
    Eye,
    EyeOff,
    Key,
    Loader2,
    RefreshCw,
    Shield,
} from 'lucide-react';
import { toast } from 'sonner';
import { AnimatePresence, motion } from 'framer-motion';

import { commandClient } from '../../../lib/command-client';
import type { SecretRecoveryStatus } from '../../../lib/bindings';
import { cn } from '../../../lib/utils';

type RecoveryKeyMode = 'cloud' | 'secrets';

interface RecoveryKeyPanelProps {
    mode?: RecoveryKeyMode;
}

export function RecoveryKeyPanel({ mode = 'cloud' }: RecoveryKeyPanelProps) {
    const secretMode = mode === 'secrets';
    const [status, setStatus] = useState<SecretRecoveryStatus | null>(null);
    const [statusError, setStatusError] = useState<string | null>(null);
    const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
    const [showKey, setShowKey] = useState(false);
    const [loading, setLoading] = useState(false);
    const [importKey, setImportKey] = useState('');
    const [importConfirmation, setImportConfirmation] = useState('');
    const [importing, setImporting] = useState(false);
    const [showImport, setShowImport] = useState(false);
    const [showRotate, setShowRotate] = useState(false);
    const [rotateConfirmation, setRotateConfirmation] = useState('');
    const [rotating, setRotating] = useState(false);

    const loadSecretStatus = async () => {
        if (!secretMode) return;
        try {
            setStatus(await commandClient.thinclawSecretRecoveryStatus());
            setStatusError(null);
        } catch (error) {
            setStatusError(String(error));
            toast.error('Failed to inspect secret encryption: ' + String(error));
        }
    };

    useEffect(() => {
        void loadSecretStatus();
    }, [secretMode]);

    useEffect(() => {
        if (!recoveryKey) return;
        const timeout = window.setTimeout(() => {
            setShowKey(false);
            setRecoveryKey(null);
        }, 60_000);
        return () => window.clearTimeout(timeout);
    }, [recoveryKey]);

    const handleGetKey = async () => {
        setLoading(true);
        try {
            const key = secretMode
                ? await commandClient.thinclawSecretRecoveryExport()
                : await commandClient.cloudGetRecoveryKey();
            setRecoveryKey(key);
            setShowKey(true);
        } catch (error) {
            toast.error('Failed to retrieve recovery key: ' + String(error));
        } finally {
            setLoading(false);
        }
    };

    const handleCopy = async () => {
        if (!recoveryKey) return;
        try {
            await navigator.clipboard.writeText(recoveryKey);
            toast.success('Recovery key copied to clipboard');
        } catch {
            toast.error('Failed to copy');
        }
    };

    const handleImport = async () => {
        if (!importKey.trim()) return;
        setImporting(true);
        try {
            if (secretMode) {
                const report = await commandClient.thinclawSecretRecoveryImport(
                    importKey.trim(),
                    importConfirmation,
                );
                toast.success(`Secret master key advanced to version ${report.new_key_version}`);
                await loadSecretStatus();
            } else {
                await commandClient.cloudImportRecoveryKey(importKey.trim());
                toast.success('Recovery key imported successfully');
            }
            setImportKey('');
            setImportConfirmation('');
            setShowImport(false);
        } catch (error) {
            toast.error('Failed to import recovery key: ' + String(error));
        } finally {
            setImporting(false);
        }
    };

    const handleRotate = async () => {
        setRotating(true);
        try {
            const report = await commandClient.thinclawSecretMasterKeyRotate(rotateConfirmation);
            setRecoveryKey(report.recovery_key);
            setShowKey(Boolean(report.recovery_key));
            setRotateConfirmation('');
            setShowRotate(false);
            toast.success(
                `Re-encrypted ${report.rotated_secrets} secrets under key version ${report.new_key_version}`,
            );
            await loadSecretStatus();
        } catch (error) {
            toast.error('Failed to rotate secret master key: ' + String(error));
        } finally {
            setRotating(false);
        }
    };

    const supported = !secretMode || status?.supported === true;
    const revealDisabled = loading || !supported;

    return (
        <div className="space-y-4 rounded-2xl border border-border/50 bg-card/40 p-5">
            <div className="flex items-center gap-3">
                <div className="rounded-xl bg-amber-500/10 p-2.5">
                    <Shield className="h-5 w-5 text-amber-500" />
                </div>
                <div>
                    <h3 className="text-base font-bold">
                        {secretMode ? 'Secret Encryption Recovery' : 'Recovery Key'}
                    </h3>
                    <p className="text-xs text-muted-foreground">
                        {secretMode
                            ? 'Back up or replace the key that encrypts the unified Desktop secret envelope.'
                            : 'Required to decrypt your cloud data on a new device. Store it safely.'}
                    </p>
                </div>
            </div>

            {secretMode && status && (
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                    <StatusValue label="Cipher" value={status.cipher} />
                    <StatusValue label="Derivation" value={status.kdf} />
                    <StatusValue label="Key version" value={status.key_version} />
                    <StatusValue label="Stored secrets" value={status.stored_secrets} />
                </div>
            )}

            {secretMode && status?.unavailable_reason && (
                <div role="status" className="rounded-xl border border-amber-500/20 bg-amber-500/5 p-3 text-xs text-amber-600 dark:text-amber-400">
                    {status.unavailable_reason}
                </div>
            )}

            {secretMode && statusError && (
                <div role="alert" className="rounded-xl border border-destructive/20 bg-destructive/5 p-3 text-xs text-destructive">
                    Secret encryption status could not be loaded: {statusError}
                </div>
            )}

            {showKey && recoveryKey ? (
                <div className="space-y-3">
                    <div className="select-all break-all rounded-xl border border-border/30 bg-muted/30 p-3 font-mono text-xs">
                        {recoveryKey}
                    </div>
                    <div className="flex gap-2">
                        <button type="button" onClick={handleCopy} className="flex h-8 items-center gap-1.5 rounded-lg border border-border/50 px-3 text-xs font-medium transition-all hover:bg-accent/50">
                            <Copy className="h-3.5 w-3.5" /> Copy
                        </button>
                        <button type="button" onClick={() => { setShowKey(false); setRecoveryKey(null); }} className="flex h-8 items-center gap-1.5 rounded-lg px-3 text-xs font-medium text-muted-foreground transition-all hover:bg-muted/50">
                            <EyeOff className="h-3.5 w-3.5" /> Hide
                        </button>
                    </div>
                    <p className="text-[11px] text-muted-foreground">This reveal clears automatically after one minute.</p>
                </div>
            ) : (
                <div className="flex flex-wrap gap-2">
                    <button type="button" onClick={handleGetKey} disabled={revealDisabled} className={cn(
                        'flex h-9 items-center gap-1.5 rounded-xl border border-amber-500/20 bg-amber-500/10 px-4 text-xs font-bold uppercase tracking-wider text-amber-600 transition-all hover:bg-amber-500/20 dark:text-amber-400',
                        revealDisabled && 'cursor-not-allowed opacity-50',
                    )}>
                        {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Eye className="h-3.5 w-3.5" />}
                        Show Recovery Key
                    </button>
                    <button type="button" onClick={() => setShowImport(!showImport)} disabled={!supported} className="flex h-9 items-center gap-1.5 rounded-xl border border-border/50 px-4 text-xs font-bold uppercase tracking-wider text-muted-foreground transition-all hover:bg-muted/50 disabled:cursor-not-allowed disabled:opacity-50">
                        <Key className="h-3.5 w-3.5" /> Import Key
                    </button>
                    {secretMode && (
                        <button type="button" onClick={() => setShowRotate(!showRotate)} disabled={!supported} className="flex h-9 items-center gap-1.5 rounded-xl border border-border/50 px-4 text-xs font-bold uppercase tracking-wider text-muted-foreground transition-all hover:bg-muted/50 disabled:cursor-not-allowed disabled:opacity-50">
                            <RefreshCw className="h-3.5 w-3.5" /> Rotate Key
                        </button>
                    )}
                </div>
            )}

            <AnimatePresence>
                {showImport && (
                    <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden">
                        <div className="space-y-2 pt-2">
                            <input type="password" autoComplete="off" value={importKey} onChange={(event) => setImportKey(event.target.value)} placeholder="Paste your recovery key here…" aria-label="Recovery key" className="h-10 w-full rounded-xl border border-border/50 bg-background/50 px-4 font-mono text-sm outline-hidden transition-all focus:border-primary/30 focus:ring-2 focus:ring-primary/20" />
                            {secretMode && (
                                <input type="text" autoComplete="off" value={importConfirmation} onChange={(event) => setImportConfirmation(event.target.value)} placeholder="Type REPLACE to confirm" aria-label="Import confirmation" className="h-10 w-full rounded-xl border border-border/50 bg-background/50 px-4 text-sm outline-hidden transition-all focus:border-primary/30 focus:ring-2 focus:ring-primary/20" />
                            )}
                            <button type="button" onClick={handleImport} disabled={importing || !importKey.trim() || (secretMode && importConfirmation !== 'REPLACE')} className="h-10 rounded-xl bg-primary px-4 text-xs font-bold uppercase tracking-wider text-primary-foreground shadow-xs transition-all hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50">
                                {importing ? <Loader2 className="h-4 w-4 animate-spin" /> : 'Import'}
                            </button>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            <AnimatePresence>
                {secretMode && showRotate && (
                    <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden">
                        <div className="space-y-2 rounded-xl border border-destructive/25 bg-destructive/5 p-3">
                            <p className="text-xs text-muted-foreground">
                                Rotation transactionally re-encrypts every stored secret. Back up the newly displayed recovery key after it succeeds.
                            </p>
                            <input type="text" autoComplete="off" value={rotateConfirmation} onChange={(event) => setRotateConfirmation(event.target.value)} placeholder="Type ROTATE to confirm" aria-label="Rotation confirmation" className="h-10 w-full rounded-xl border border-border/50 bg-background/70 px-4 text-sm outline-hidden transition-all focus:border-primary/30 focus:ring-2 focus:ring-primary/20" />
                            <button type="button" onClick={handleRotate} disabled={rotating || rotateConfirmation !== 'ROTATE'} className="flex h-10 items-center gap-2 rounded-xl bg-destructive px-4 text-xs font-bold uppercase tracking-wider text-destructive-foreground transition-all hover:bg-destructive/90 disabled:cursor-not-allowed disabled:opacity-50">
                                {rotating ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                                Rotate Master Key
                            </button>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            <div className="flex items-start gap-2 rounded-xl border border-amber-500/10 bg-amber-500/5 p-3 text-[11px] leading-relaxed text-amber-600 dark:text-amber-400">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>
                    <strong>Important:</strong>{' '}
                    {secretMode
                        ? 'The key does not contain your credentials. Keep it with a secure backup of the encrypted Keychain envelope; importing it replaces the active key after re-encrypting current secrets.'
                        : 'Without this key, encrypted cloud data is permanently unrecoverable. Store it in a password manager or another secure place.'}
                </span>
            </div>
        </div>
    );
}

function StatusValue({ label, value }: { label: string; value: string | number }) {
    return (
        <div className="rounded-lg bg-muted/35 p-2.5">
            <div className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</div>
            <div className="mt-1 text-sm font-semibold">{value}</div>
        </div>
    );
}
