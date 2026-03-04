import { useState, useCallback, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Shield,
    UserCheck,
    UserPlus,
    Clock,
    Trash2 as _Trash2,
    RefreshCw,
    CheckCircle2,
    AlertCircle,
    Loader2,
} from 'lucide-react';
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
        <div style={{
            display: 'flex',
            flexDirection: 'column',
            height: '100%',
            background: 'linear-gradient(135deg, #0d1117 0%, #161b22 100%)',
            color: '#e6edf3',
            overflow: 'hidden',
        }}>
            {/* Header */}
            <div style={{
                padding: '20px 24px 16px',
                borderBottom: '1px solid rgba(255,255,255,0.06)',
                display: 'flex',
                alignItems: 'center',
                gap: 12,
            }}>
                <div style={{
                    width: 36,
                    height: 36,
                    borderRadius: 10,
                    background: 'linear-gradient(135deg, #238636, #2ea043)',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                }}>
                    <Shield size={18} color="#fff" />
                </div>
                <div>
                    <h2 style={{ margin: 0, fontSize: 18, fontWeight: 600 }}>
                        DM Pairing
                    </h2>
                    <p style={{ margin: 0, fontSize: 12, color: '#8b949e' }}>
                        Manage approved senders for messaging channels
                    </p>
                </div>
                <div style={{ flex: 1 }} />
                <button
                    onClick={fetchPairings}
                    style={{
                        background: 'rgba(255,255,255,0.06)',
                        border: '1px solid rgba(255,255,255,0.1)',
                        borderRadius: 8,
                        padding: '6px 12px',
                        color: '#8b949e',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: 6,
                        fontSize: 13,
                    }}
                >
                    <RefreshCw size={14} className={loading ? 'spin' : ''} />
                    Refresh
                </button>
            </div>

            {/* Channel Tabs */}
            <div style={{
                display: 'flex',
                gap: 4,
                padding: '12px 24px 8px',
                flexWrap: 'wrap',
            }}>
                {CHANNELS.map(ch => (
                    <button
                        key={ch}
                        onClick={() => setSelectedChannel(ch)}
                        style={{
                            background: selectedChannel === ch
                                ? 'rgba(56,139,253,0.15)'
                                : 'rgba(255,255,255,0.04)',
                            border: selectedChannel === ch
                                ? '1px solid rgba(56,139,253,0.4)'
                                : '1px solid rgba(255,255,255,0.06)',
                            borderRadius: 6,
                            padding: '5px 12px',
                            color: selectedChannel === ch ? '#58a6ff' : '#8b949e',
                            cursor: 'pointer',
                            fontSize: 13,
                            fontWeight: selectedChannel === ch ? 600 : 400,
                            textTransform: 'capitalize',
                            transition: 'all 0.15s ease',
                        }}
                    >
                        {ch}
                    </button>
                ))}
            </div>

            {/* Approve Code Input */}
            <div style={{
                padding: '8px 24px 16px',
                display: 'flex',
                gap: 8,
                alignItems: 'center',
            }}>
                <input
                    type="text"
                    value={approveCode}
                    onChange={e => setApproveCode(e.target.value.toUpperCase())}
                    placeholder="Enter pairing code..."
                    onKeyDown={e => e.key === 'Enter' && handleApprove()}
                    style={{
                        flex: 1,
                        padding: '8px 12px',
                        borderRadius: 8,
                        border: '1px solid rgba(255,255,255,0.1)',
                        background: 'rgba(255,255,255,0.04)',
                        color: '#e6edf3',
                        fontSize: 14,
                        fontFamily: 'monospace',
                        letterSpacing: '0.1em',
                        outline: 'none',
                    }}
                />
                <button
                    onClick={handleApprove}
                    disabled={!approveCode.trim() || approving}
                    style={{
                        background: approveCode.trim()
                            ? 'linear-gradient(135deg, #238636, #2ea043)'
                            : 'rgba(255,255,255,0.04)',
                        border: 'none',
                        borderRadius: 8,
                        padding: '8px 16px',
                        color: approveCode.trim() ? '#fff' : '#484f58',
                        cursor: approveCode.trim() ? 'pointer' : 'default',
                        fontSize: 13,
                        fontWeight: 600,
                        display: 'flex',
                        alignItems: 'center',
                        gap: 6,
                    }}
                >
                    {approving ? (
                        <Loader2 size={14} className="spin" />
                    ) : (
                        <UserPlus size={14} />
                    )}
                    Approve
                </button>
            </div>

            {/* Content */}
            <div style={{
                flex: 1,
                overflow: 'auto',
                padding: '0 24px 24px',
            }}>
                {loading ? (
                    <div style={{
                        display: 'flex',
                        justifyContent: 'center',
                        padding: 40,
                    }}>
                        <Loader2
                            size={24}
                            className="spin"
                            color="#8b949e"
                        />
                    </div>
                ) : (
                    <>
                        {/* Pending Requests */}
                        {pendingPairings.length > 0 && (
                            <div style={{ marginBottom: 24 }}>
                                <h3 style={{
                                    fontSize: 13,
                                    fontWeight: 600,
                                    color: '#d29922',
                                    textTransform: 'uppercase',
                                    letterSpacing: '0.05em',
                                    marginBottom: 8,
                                    display: 'flex',
                                    alignItems: 'center',
                                    gap: 6,
                                }}>
                                    <Clock size={14} />
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
                                            style={{
                                                background: 'rgba(210,153,34,0.06)',
                                                border: '1px solid rgba(210,153,34,0.2)',
                                                borderRadius: 10,
                                                padding: '12px 16px',
                                                marginBottom: 6,
                                                display: 'flex',
                                                alignItems: 'center',
                                                gap: 12,
                                            }}
                                        >
                                            <AlertCircle
                                                size={16}
                                                color="#d29922"
                                            />
                                            <div style={{ flex: 1 }}>
                                                <div style={{
                                                    fontSize: 14,
                                                    fontWeight: 500,
                                                    fontFamily: 'monospace',
                                                }}>
                                                    {p.user_id}
                                                </div>
                                                {p.paired_at && (
                                                    <div style={{
                                                        fontSize: 11,
                                                        color: '#8b949e',
                                                        marginTop: 2,
                                                    }}>
                                                        Requested: {new Date(p.paired_at).toLocaleString()}
                                                    </div>
                                                )}
                                            </div>
                                            <span style={{
                                                fontSize: 11,
                                                fontWeight: 600,
                                                color: '#d29922',
                                                background: 'rgba(210,153,34,0.1)',
                                                padding: '2px 8px',
                                                borderRadius: 4,
                                            }}>
                                                PENDING
                                            </span>
                                        </motion.div>
                                    ))}
                                </AnimatePresence>
                            </div>
                        )}

                        {/* Approved Senders */}
                        <div>
                            <h3 style={{
                                fontSize: 13,
                                fontWeight: 600,
                                color: '#3fb950',
                                textTransform: 'uppercase',
                                letterSpacing: '0.05em',
                                marginBottom: 8,
                                display: 'flex',
                                alignItems: 'center',
                                gap: 6,
                            }}>
                                <UserCheck size={14} />
                                Approved ({activePairings.length})
                            </h3>
                            {activePairings.length === 0 && pendingPairings.length === 0 ? (
                                <div style={{
                                    textAlign: 'center',
                                    padding: 40,
                                    color: '#484f58',
                                    fontSize: 14,
                                }}>
                                    No pairings for{' '}
                                    <span style={{
                                        textTransform: 'capitalize',
                                        color: '#8b949e',
                                    }}>
                                        {selectedChannel}
                                    </span>
                                    . Users can pair by sending a DM to the agent.
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
                                            style={{
                                                background: 'rgba(63,185,80,0.04)',
                                                border: '1px solid rgba(63,185,80,0.15)',
                                                borderRadius: 10,
                                                padding: '12px 16px',
                                                marginBottom: 6,
                                                display: 'flex',
                                                alignItems: 'center',
                                                gap: 12,
                                            }}
                                        >
                                            <CheckCircle2
                                                size={16}
                                                color="#3fb950"
                                            />
                                            <div style={{ flex: 1 }}>
                                                <div style={{
                                                    fontSize: 14,
                                                    fontWeight: 500,
                                                    fontFamily: 'monospace',
                                                }}>
                                                    {p.user_id}
                                                </div>
                                            </div>
                                            <span style={{
                                                fontSize: 11,
                                                fontWeight: 600,
                                                color: '#3fb950',
                                                background: 'rgba(63,185,80,0.08)',
                                                padding: '2px 8px',
                                                borderRadius: 4,
                                            }}>
                                                ACTIVE
                                            </span>
                                        </motion.div>
                                    ))}
                                </AnimatePresence>
                            )}
                        </div>
                    </>
                )}
            </div>

            {/* Spin animation CSS */}
            <style>{`
                @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
                .spin { animation: spin 1s linear infinite; }
            `}</style>
        </div>
    );
}
