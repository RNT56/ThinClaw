import { motion } from 'framer-motion';
import { KeyRound, Lock, ShieldCheck } from 'lucide-react';
import { useState } from 'react';
import { setRepoProjectCredential } from '../../lib/thinclaw';
import { toast } from 'sonner';

interface CredentialPromptCardProps {
    promptId: string;
    secretName: string;
    provider: string;
    reason: string;
    onResolved?: () => void;
}

/**
 * Inline secure credential card. The agent emits only a prompt; the operator's
 * typed value goes straight to the encrypted secrets store via
 * `setRepoProjectCredential` — it never travels through the engine or the model.
 */
export function CredentialPromptCard({
    promptId,
    secretName,
    provider,
    reason,
    onResolved,
}: CredentialPromptCardProps) {
    const [value, setValue] = useState('');
    const [isSaving, setIsSaving] = useState(false);
    const [stored, setStored] = useState(false);

    const submit = async () => {
        if (!value.trim()) {
            toast.error('Enter the credential value');
            return;
        }
        setIsSaving(true);
        try {
            const res = await setRepoProjectCredential(secretName, value);
            if (res.ok) {
                setValue('');
                setStored(true);
                toast.success(`Stored ${secretName} securely`);
                onResolved?.();
            } else {
                toast.error(res.unavailable?.reason || `Failed to store ${secretName}`);
            }
        } catch (error) {
            console.error(error);
            toast.error('Unexpected error storing credential');
        } finally {
            setIsSaving(false);
        }
    };

    if (stored) {
        return (
            <motion.div
                initial={{ opacity: 1, scale: 1 }}
                animate={{ opacity: 0.7, scale: 0.98 }}
                className="w-full border rounded-xl overflow-hidden my-4 bg-emerald-500/5 border-emerald-500/20"
            >
                <div className="px-4 py-2 flex items-center gap-2 bg-emerald-500/10">
                    <ShieldCheck className="w-4 h-4 text-emerald-500" />
                    <span className="text-xs font-bold uppercase tracking-widest text-emerald-500">
                        Stored securely
                    </span>
                    <code className="ml-auto text-[10px] bg-black/20 px-1.5 py-0.5 rounded text-muted-foreground font-mono">
                        {secretName}
                    </code>
                </div>
            </motion.div>
        );
    }

    return (
        <motion.div
            key={promptId}
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            className="w-full bg-amber-500/10 border border-amber-500/30 rounded-xl overflow-hidden shadow-lg my-4"
        >
            <div className="bg-amber-500/20 px-4 py-2 border-b border-amber-500/20 flex items-center gap-2">
                <KeyRound className="w-4 h-4 text-amber-500" />
                <span className="text-xs font-bold uppercase tracking-widest text-amber-500">
                    Credential requested
                </span>
                <code className="ml-auto text-[10px] bg-black/20 px-1.5 py-0.5 rounded text-muted-foreground font-mono">
                    {provider}
                </code>
            </div>
            <div className="p-4 space-y-3">
                <p className="text-sm text-gray-200">{reason}</p>
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                    <Lock className="w-3 h-3" />
                    Stored encrypted as <code className="font-mono text-amber-400">{secretName}</code> — never sent to the model.
                </div>
                <input
                    type="password"
                    autoComplete="off"
                    value={value}
                    onChange={(event) => setValue(event.target.value)}
                    onKeyDown={(event) => {
                        if (event.key === 'Enter') submit();
                    }}
                    placeholder={`${secretName} value`}
                    className="w-full rounded-lg border border-white/10 bg-black/40 px-3 py-2 text-xs outline-none placeholder:text-muted-foreground focus:border-amber-500/40"
                />
                <button
                    onClick={submit}
                    disabled={isSaving}
                    className="w-full flex items-center justify-center gap-1.5 py-2 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-lg font-bold text-xs transition-colors shadow-md"
                >
                    <KeyRound className="w-3.5 h-3.5" />
                    Store securely
                </button>
            </div>
        </motion.div>
    );
}
