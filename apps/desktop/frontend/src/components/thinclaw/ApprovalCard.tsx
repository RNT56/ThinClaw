
import { motion } from 'framer-motion';
import { Shield, Check, X, Info, ShieldCheck } from 'lucide-react';
import { useState } from 'react';
import { resolveThinClawApproval } from '../../lib/thinclaw';
import { toast } from 'sonner';

interface ApprovalCardProps {
    approvalId: string;
    tool: string;
    input: any;
    onResolved?: (approved: boolean) => void;
}

export function ApprovalCard({ approvalId, tool, input, onResolved }: ApprovalCardProps) {
    const [isResolving, setIsResolving] = useState(false);
    const [resolvedDecision, setResolvedDecision] = useState<string | null>(null);

    const handleAction = async (approved: boolean, allowSession: boolean = false) => {
        setIsResolving(true);
        try {
            const res = await resolveThinClawApproval(approvalId, approved, allowSession);
            if (res.ok) {
                const label = !approved
                    ? 'Action denied'
                    : allowSession
                        ? 'Approved for session'
                        : 'Action approved';
                toast.success(label);
                setResolvedDecision(
                    !approved ? 'denied' : allowSession ? 'session' : 'once'
                );
                onResolved?.(approved);
            } else {
                toast.error(res.message || 'Failed to resolve approval');
            }
        } catch (e) {
            console.error(e);
            toast.error('Unexpected error resolving approval');
        } finally {
            setIsResolving(false);
        }
    };

    // Show resolved state
    if (resolvedDecision) {
        const isApproved = resolvedDecision !== 'denied';
        return (
            <motion.div
                initial={{ opacity: 1, scale: 1 }}
                animate={{ opacity: 0.7, scale: 0.98 }}
                className={`w-full border rounded-xl overflow-hidden my-4 ${isApproved
                        ? 'bg-emerald-500/5 border-emerald-500/20'
                        : 'bg-red-500/5 border-red-500/20'
                    }`}
            >
                <div className={`px-4 py-2 flex items-center gap-2 ${isApproved ? 'bg-emerald-500/10' : 'bg-red-500/10'
                    }`}>
                    {isApproved ? (
                        <ShieldCheck className="w-4 h-4 text-emerald-500" />
                    ) : (
                        <X className="w-4 h-4 text-red-400" />
                    )}
                    <span className={`text-xs font-bold uppercase tracking-widest ${isApproved ? 'text-emerald-500' : 'text-red-400'
                        }`}>
                        {resolvedDecision === 'session'
                            ? 'Approved for Session'
                            : resolvedDecision === 'once'
                                ? 'Approved (Once)'
                                : 'Denied'}
                    </span>
                    <code className="ml-auto text-[10px] bg-black/20 px-1.5 py-0.5 rounded text-muted-foreground font-mono">{tool}</code>
                </div>
            </motion.div>
        );
    }

    return (
        <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            className="w-full bg-amber-500/10 border border-amber-500/30 rounded-xl overflow-hidden shadow-lg my-4"
        >
            <div className="bg-amber-500/20 px-4 py-2 border-b border-amber-500/20 flex items-center gap-2">
                <Shield className="w-4 h-4 text-amber-500" />
                <span className="text-xs font-bold uppercase tracking-widest text-amber-500">Security Approval Required</span>
            </div>
            <div className="p-4 space-y-4">
                <div>
                    <div className="flex items-center gap-2 mb-2">
                        <Info className="w-3.5 h-3.5 text-muted-foreground" />
                        <span className="text-sm font-medium text-gray-200">The agent wants to run:</span>
                        <code className="text-xs bg-black/40 px-1.5 py-0.5 rounded text-amber-400 font-mono font-bold">{tool}</code>
                    </div>
                    <pre className="text-xs font-mono bg-black/60 p-3 rounded-lg border border-white/5 text-gray-400 overflow-x-auto whitespace-pre-wrap max-h-48">
                        {typeof input === 'string' ? input : JSON.stringify(input, null, 2)}
                    </pre>
                </div>

                {/* 3-tier approval buttons */}
                <div className="flex items-center gap-2">
                    {/* Allow Once */}
                    <button
                        onClick={() => handleAction(true, false)}
                        disabled={isResolving}
                        className="flex-1 flex items-center justify-center gap-1.5 py-2 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white rounded-lg font-bold text-xs transition-colors shadow-md"
                    >
                        <Check className="w-3.5 h-3.5" />
                        Allow Once
                    </button>

                    {/* Allow Session */}
                    <button
                        onClick={() => handleAction(true, true)}
                        disabled={isResolving}
                        className="flex-1 flex items-center justify-center gap-1.5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg font-bold text-xs transition-colors shadow-md"
                    >
                        <ShieldCheck className="w-3.5 h-3.5" />
                        Allow Session
                    </button>

                    {/* Deny */}
                    <button
                        onClick={() => handleAction(false)}
                        disabled={isResolving}
                        className="flex-[0.7] flex items-center justify-center gap-1.5 py-2 bg-red-600/20 hover:bg-red-600/30 disabled:opacity-50 text-red-400 border border-red-600/30 rounded-lg font-bold text-xs transition-colors"
                    >
                        <X className="w-3.5 h-3.5" />
                        Deny
                    </button>
                </div>

                {/* Info text about session approval */}
                <p className="text-[10px] text-muted-foreground/50 leading-relaxed">
                    <strong>Allow Session</strong> grants permission for this tool until the engine restarts.
                    <strong> Allow Once</strong> permits only this specific request.
                </p>
            </div>
        </motion.div>
    );
}
