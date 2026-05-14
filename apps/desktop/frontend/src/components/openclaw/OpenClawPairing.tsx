import { useState, useCallback, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Shield,
    UserCheck,
    UserPlus,
    Clock,
    RefreshCw,
    CheckCircle2,
    AlertCircle,
    Loader2,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import { toast } from 'sonner';
import * as openclawApi from '../../lib/openclaw';

type PairingItem = openclawApi.PairingItem;

const CHANNELS = ['telegram', 'signal', 'discord', 'whatsapp', 'nostr', 'slack'];

export function OpenClawPairing() {
    const [selectedChannel, setSelectedChannel] = useState('telegram');
    const [pairings, setPairings] = useState<PairingItem[]>([]);
    const [loading, setLoading] = useState(false);
    const [approveCode, setApproveCode] = useState('');
    const [approving, setApproving] = useState(false);

    const fetchPairings = useCallback(async () => {
        setLoading(true);
        try {
            const resp = await openclawApi.listPairings(selectedChannel);
            setPairings(resp.pairings);
        } catch (err) {
            console.error('Failed to load pairings:', err);
            setPairings([]);
        } finally {
            setLoading(false);
        }
    }, [selectedChannel]);

    useEffect(() => {
        fetchPairings();
    }, [fetchPairings]);

    const handleApprove = async () => {
        if (!approveCode.trim()) return;
        setApproving(true);
        try {
            await openclawApi.approvePairing(selectedChannel, approveCode.trim());
            toast.success('Pairing approved');
            setApproveCode('');
            fetchPairings();
        } catch (err) {
            toast.error(`Failed to approve: ${err}`);
        } finally {
            setApproving(false);
        }
    };

    const activePairings = pairings.filter(p => p.status === 'active');
    const pendingPairings = pairings.filter(p => p.status === 'pending');

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 overflow-y-auto p-8 space-y-6 max-w-4xl mx-auto"
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-primary/10">
                        <Shield className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-2xl font-bold tracking-tight">DM Pairing</h1>
                        <p className="text-xs text-muted-foreground mt-0.5">
                            Manage approved senders for messaging channels
                        </p>
                    </div>
                </div>
                <button
                    onClick={fetchPairings}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 transition-all"
                >
                    <RefreshCw className={cn("w-3.5 h-3.5", loading && "animate-spin")} />
                    Refresh
                </button>
            </div>

            {/* Channel Tabs */}
            <div className="flex gap-1.5 flex-wrap">
                {CHANNELS.map(ch => (
                    <button
                        key={ch}
                        onClick={() => setSelectedChannel(ch)}
                        className={cn(
                            "px-3 py-1.5 rounded-lg text-xs font-medium capitalize transition-all",
                            selectedChannel === ch
                                ? "bg-primary/15 text-primary"
                                : "bg-white/[0.03] text-muted-foreground hover:text-foreground hover:bg-white/5"
                        )}
                    >
                        {ch}
                    </button>
                ))}
            </div>

            {/* Approve Code Input */}
            <div className="flex gap-2 items-center">
                <input
                    type="text"
                    value={approveCode}
                    onChange={e => setApproveCode(e.target.value.toUpperCase())}
                    placeholder="Enter pairing code..."
                    onKeyDown={e => e.key === 'Enter' && handleApprove()}
                    className="flex-1 px-3 py-2 rounded-lg bg-white/[0.03] text-foreground text-sm font-mono tracking-wider placeholder:text-muted-foreground/40 outline-none focus:ring-1 focus:ring-primary/30 transition-all"
                />
                <button
                    onClick={handleApprove}
                    disabled={!approveCode.trim() || approving}
                    className={cn(
                        "flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-bold transition-all",
                        approveCode.trim()
                            ? "bg-primary/15 text-primary hover:bg-primary/25"
                            : "bg-white/[0.03] text-muted-foreground/30 cursor-default"
                    )}
                >
                    {approving ? (
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                    ) : (
                        <UserPlus className="w-3.5 h-3.5" />
                    )}
                    Approve
                </button>
            </div>

            {/* Content */}
            {loading ? (
                <div className="flex justify-center py-16">
                    <Loader2 className="w-6 h-6 animate-spin text-muted-foreground" />
                </div>
            ) : (
                <div className="space-y-6">
                    {/* Pending Requests */}
                    {pendingPairings.length > 0 && (
                        <div>
                            <h3 className="text-xs font-bold uppercase tracking-widest text-amber-500 mb-3 flex items-center gap-1.5">
                                <Clock className="w-3.5 h-3.5" />
                                Pending ({pendingPairings.length})
                            </h3>
                            <AnimatePresence>
                                {pendingPairings.map((p, i) => (
                                    <motion.div
                                        key={`pending-${p.user_id}`}
                                        initial={{ opacity: 0, y: 8 }}
                                        animate={{ opacity: 1, y: 0 }}
                                        exit={{ opacity: 0, y: -8 }}
                                        transition={{ delay: i * 0.05 }}
                                        className="flex items-center gap-3 px-4 py-3 mb-1.5 rounded-xl bg-amber-500/5"
                                    >
                                        <AlertCircle className="w-4 h-4 text-amber-500 shrink-0" />
                                        <div className="flex-1 min-w-0">
                                            <p className="text-sm font-medium font-mono truncate">{p.user_id}</p>
                                            {p.paired_at && (
                                                <p className="text-[10px] text-muted-foreground mt-0.5">
                                                    Requested: {new Date(p.paired_at).toLocaleString()}
                                                </p>
                                            )}
                                        </div>
                                        <span className="text-[9px] font-bold uppercase tracking-wider text-amber-500 bg-amber-500/10 px-2 py-0.5 rounded shrink-0">
                                            Pending
                                        </span>
                                    </motion.div>
                                ))}
                            </AnimatePresence>
                        </div>
                    )}

                    {/* Approved Senders */}
                    <div>
                        <h3 className="text-xs font-bold uppercase tracking-widest text-green-500 mb-3 flex items-center gap-1.5">
                            <UserCheck className="w-3.5 h-3.5" />
                            Approved ({activePairings.length})
                        </h3>

                        {activePairings.length === 0 && pendingPairings.length === 0 ? (
                            <div className="text-center py-16">
                                <UserPlus className="w-8 h-8 text-muted-foreground/20 mx-auto mb-3" />
                                <p className="text-sm text-muted-foreground">
                                    No pairings for{' '}
                                    <span className="capitalize text-foreground/70">{selectedChannel}</span>.
                                </p>
                                <p className="text-xs text-muted-foreground/60 mt-1">
                                    Users can pair by sending a DM to the agent.
                                </p>
                            </div>
                        ) : (
                            <AnimatePresence>
                                {activePairings.map((p, i) => (
                                    <motion.div
                                        key={`active-${p.user_id}`}
                                        initial={{ opacity: 0, y: 8 }}
                                        animate={{ opacity: 1, y: 0 }}
                                        exit={{ opacity: 0, y: -8 }}
                                        transition={{ delay: i * 0.03 }}
                                        className="flex items-center gap-3 px-4 py-3 mb-1.5 rounded-xl bg-green-500/5"
                                    >
                                        <CheckCircle2 className="w-4 h-4 text-green-500 shrink-0" />
                                        <div className="flex-1 min-w-0">
                                            <p className="text-sm font-medium font-mono truncate">{p.user_id}</p>
                                        </div>
                                        <span className="text-[9px] font-bold uppercase tracking-wider text-green-500 bg-green-500/10 px-2 py-0.5 rounded shrink-0">
                                            Active
                                        </span>
                                    </motion.div>
                                ))}
                            </AnimatePresence>
                        )}
                    </div>
                </div>
            )}
        </motion.div>
    );
}
