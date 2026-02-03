
import { motion } from 'framer-motion';
import { Shield, Check, X, Info } from 'lucide-react';
import { useState } from 'react';
import { resolveClawdbotApproval } from '../../lib/clawdbot';
import { toast } from 'sonner';

interface ApprovalCardProps {
    approvalId: string;
    tool: string;
    input: any;
    onResolved?: (approved: boolean) => void;
}

export function ApprovalCard({ approvalId, tool, input, onResolved }: ApprovalCardProps) {
    const [isResolving, setIsResolving] = useState(false);

    const handleAction = async (approved: boolean) => {
        setIsResolving(true);
        try {
            const res = await resolveClawdbotApproval(approvalId, approved);
            if (res.ok) {
                toast.success(approved ? 'Action approved' : 'Action denied');
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

                <div className="flex items-center gap-3">
                    <button
                        onClick={() => handleAction(true)}
                        disabled={isResolving}
                        className="flex-1 flex items-center justify-center gap-2 py-2 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white rounded-lg font-bold text-xs transition-colors shadow-md"
                    >
                        <Check className="w-4 h-4" />
                        Approve
                    </button>
                    <button
                        onClick={() => handleAction(false)}
                        disabled={isResolving}
                        className="flex-1 flex items-center justify-center gap-2 py-2 bg-red-600/20 hover:bg-red-600/30 disabled:opacity-50 text-red-400 border border-red-600/30 rounded-lg font-bold text-xs transition-colors"
                    >
                        <X className="w-4 h-4" />
                        Deny
                    </button>
                </div>
            </div>
        </motion.div>
    );
}
