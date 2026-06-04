/**
 * RecoveryKeyPanel — shows / copies / imports the encryption recovery key.
 */
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
    Shield, Loader2, AlertTriangle,
    Copy, Eye, EyeOff, Key
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../../lib/utils';
import { AnimatePresence, motion } from 'framer-motion';

export function RecoveryKeyPanel() {
    const [recoveryKey, setRecoveryKey] = useState<string | null>(null);
    const [showKey, setShowKey] = useState(false);
    const [loading, setLoading] = useState(false);
    const [importKey, setImportKey] = useState('');
    const [importing, setImporting] = useState(false);
    const [showImport, setShowImport] = useState(false);

    const handleGetKey = async () => {
        setLoading(true);
        try {
            const key = await invoke<string>('cloud_get_recovery_key');
            setRecoveryKey(key);
            setShowKey(true);
        } catch (e) {
            toast.error('Failed to retrieve recovery key: ' + String(e));
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
            await invoke('cloud_import_recovery_key', { recoveryKey: importKey.trim() });
            toast.success('Recovery key imported successfully');
            setImportKey('');
            setShowImport(false);
        } catch (e) {
            toast.error('Failed to import recovery key: ' + String(e));
        } finally {
            setImporting(false);
        }
    };

    return (
        <div className="p-5 rounded-2xl border border-border/50 bg-card/40 space-y-4">
            <div className="flex items-center gap-3">
                <div className="p-2.5 rounded-xl bg-amber-500/10">
                    <Shield className="w-5 h-5 text-amber-500" />
                </div>
                <div>
                    <h3 className="font-bold text-base">Recovery Key</h3>
                    <p className="text-xs text-muted-foreground">
                        Required to decrypt your data on a new device. Store this safely — it cannot be regenerated.
                    </p>
                </div>
            </div>

            {showKey && recoveryKey ? (
                <div className="space-y-3">
                    <div className="p-3 rounded-xl bg-muted/30 border border-border/30 font-mono text-xs break-all select-all">
                        {recoveryKey}
                    </div>
                    <div className="flex gap-2">
                        <button onClick={handleCopy} className="flex items-center gap-1.5 px-3 h-8 rounded-lg text-xs font-medium border border-border/50 hover:bg-accent/50 transition-all">
                            <Copy className="w-3.5 h-3.5" /> Copy
                        </button>
                        <button onClick={() => { setShowKey(false); setRecoveryKey(null); }} className="flex items-center gap-1.5 px-3 h-8 rounded-lg text-xs font-medium text-muted-foreground hover:bg-muted/50 transition-all">
                            <EyeOff className="w-3.5 h-3.5" /> Hide
                        </button>
                    </div>
                </div>
            ) : (
                <div className="flex gap-2">
                    <button onClick={handleGetKey} disabled={loading} className={cn(
                        'flex items-center gap-1.5 px-4 h-9 rounded-xl text-xs font-bold uppercase tracking-wider',
                        'bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20',
                        'hover:bg-amber-500/20 transition-all',
                        loading && 'opacity-50 cursor-wait'
                    )}>
                        {loading ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Eye className="w-3.5 h-3.5" />}
                        Show Recovery Key
                    </button>
                    <button onClick={() => setShowImport(!showImport)} className="flex items-center gap-1.5 px-4 h-9 rounded-xl text-xs font-bold uppercase tracking-wider
                        border border-border/50 text-muted-foreground hover:bg-muted/50 transition-all">
                        <Key className="w-3.5 h-3.5" /> Import Key
                    </button>
                </div>
            )}

            <AnimatePresence>
                {showImport && (
                    <motion.div initial={{ height: 0, opacity: 0 }} animate={{ height: 'auto', opacity: 1 }} exit={{ height: 0, opacity: 0 }} className="overflow-hidden">
                        <div className="flex gap-2 pt-2">
                            <input type="text" value={importKey} onChange={e => setImportKey(e.target.value)} placeholder="Paste your recovery key here…"
                                className="flex-1 h-10 rounded-xl border border-border/50 bg-background/50 px-4 text-sm font-mono focus:ring-2 focus:ring-primary/20 focus:border-primary/30 outline-none transition-all" />
                            <button onClick={handleImport} disabled={importing || !importKey.trim()} className={cn(
                                'px-4 h-10 rounded-xl bg-primary text-primary-foreground text-xs font-bold uppercase tracking-wider',
                                'hover:bg-primary/90 transition-all shadow-sm',
                                (importing || !importKey.trim()) && 'opacity-50 cursor-not-allowed'
                            )}>
                                {importing ? <Loader2 className="w-4 h-4 animate-spin" /> : 'Import'}
                            </button>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>

            <div className="flex items-start gap-2 p-3 rounded-xl bg-amber-500/5 border border-amber-500/10 text-[11px] text-amber-600 dark:text-amber-400 leading-relaxed">
                <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                <span>
                    <strong>Important:</strong> Without this key, your encrypted cloud data is <strong>permanently unrecoverable</strong>.
                    Store it in a password manager or write it down and keep it in a secure place.
                </span>
            </div>
        </div>
    );
}
