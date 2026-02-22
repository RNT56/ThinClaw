import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    Users,
    Activity,
    Globe,
    RefreshCw,
    Clock,
    Info,
    Search,
    Monitor,
    Circle,
    Terminal,
    ChevronRight,
    User
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';

interface PresenceItemProps {
    type: 'instance' | 'node';
    data: any;
}

function PresenceItem({ type, data }: PresenceItemProps) {
    const isInstance = type === 'instance';

    return (
        <div className="p-4 rounded-xl bg-card border border-white/5 hover:bg-white/[0.04] transition-all group">
            <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-3">
                    <div className={cn(
                        "p-2 rounded-lg",
                        isInstance ? "bg-primary/10 text-primary" : "bg-blue-500/10 text-blue-400"
                    )}>
                        {isInstance ? <Monitor className="w-4 h-4" /> : <Globe className="w-4 h-4" />}
                    </div>
                    <div>
                        <h4 className="font-bold text-sm tracking-tight">{data.id || data.pubkey?.slice(0, 12)}</h4>
                        <p className="text-[10px] text-muted-foreground uppercase font-bold tracking-tighter">
                            {isInstance ? 'Active Instance' : 'Remote Node'}
                        </p>
                    </div>
                </div>
                <div className="flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-green-500/10 border border-green-500/20">
                    <Circle className="w-1.5 h-1.5 fill-green-500 text-green-500 animate-pulse" />
                    <span className="text-[9px] font-bold text-green-500 uppercase tracking-widest">Live</span>
                </div>
            </div>

            <div className="space-y-2">
                {isInstance ? (
                    <>
                        <div className="flex items-center justify-between text-[10px]">
                            <span className="text-muted-foreground flex items-center gap-1.5 uppercase font-bold tracking-tighter">
                                <Terminal className="w-3 h-3" /> Runtime
                            </span>
                            <span className="font-mono text-foreground/80">{data.runtime || 'Native'}</span>
                        </div>
                        <div className="flex items-center justify-between text-[10px]">
                            <span className="text-muted-foreground flex items-center gap-1.5 uppercase font-bold tracking-tighter">
                                <Activity className="w-3 h-3" /> Status
                            </span>
                            <span className="px-1.5 py-0.5 rounded bg-white/5 text-foreground/70 font-bold tracking-tighter">IDLE</span>
                        </div>
                    </>
                ) : (
                    <>
                        <div className="flex items-center justify-between text-[10px]">
                            <span className="text-muted-foreground flex items-center gap-1.5 uppercase font-bold tracking-tighter">
                                <ChevronRight className="w-3 h-3" /> Address
                            </span>
                            <span className="font-mono text-foreground/80">{data.addr || 'Unknown'}</span>
                        </div>
                        <div className="flex items-center justify-between text-[10px]">
                            <span className="text-muted-foreground flex items-center gap-1.5 uppercase font-bold tracking-tighter">
                                <Clock className="w-3 h-3" /> Connected
                            </span>
                            <span className="text-foreground/80 font-bold tracking-tighter">RECENT</span>
                        </div>
                    </>
                )}
            </div>

            <div className="mt-4 pt-3 border-t border-white/5 flex items-center justify-between">
                <div className="flex -space-x-1.5">
                    {[1, 2].map(i => (
                        <div key={i} className="w-5 h-5 rounded-full bg-muted border border-background flex items-center justify-center">
                            <User className="w-2.5 h-2.5 text-muted-foreground" />
                        </div>
                    ))}
                </div>
                <button className="text-[9px] font-bold uppercase tracking-widest text-primary hover:underline">
                    View Details
                </button>
            </div>
        </div>
    );
}

export function OpenClawPresence() {
    const [presence, setPresence] = useState<any>(null);
    const [filter, setFilter] = useState<'all' | 'instances' | 'nodes'>('all');
    const [isLoading, setIsLoading] = useState(true);
    const [search, setSearch] = useState('');

    const fetchData = async () => {
        try {
            const data = await openclaw.getOpenClawSystemPresence();
            setPresence(data);
        } catch (e) {
            console.error('Failed to fetch presence:', e);
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 10000);
        return () => clearInterval(interval);
    }, []);

    const instances = presence?.instances || [];
    const nodes = presence?.nodes || [];

    const filteredItems = [
        ...instances.map((i: any) => ({ ...i, p_type: 'instance' })),
        ...nodes.map((n: any) => ({ ...n, p_type: 'node' }))
    ].filter(item => {
        const matchesFilter = filter === 'all' || (filter === 'instances' ? item.p_type === 'instance' : item.p_type === 'node');
        const matchesSearch = !search || (item.id || item.pubkey || '').toLowerCase().includes(search.toLowerCase());
        return matchesFilter && matchesSearch;
    });

    return (
        <motion.div
            initial={{ opacity: 0, scale: 0.98 }}
            animate={{ opacity: 1, scale: 1 }}
            className="flex-1 p-8 space-y-8 max-w-6xl mx-auto"
        >
            <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
                <div>
                    <h1 className="text-3xl font-bold tracking-tight">Active Presence</h1>
                    <p className="text-muted-foreground mt-1 text-sm">Real-time mesh network visibility and node discovery.</p>
                </div>

                <div className="flex items-center gap-3">
                    <div className="relative">
                        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
                        <input
                            type="text"
                            placeholder="Filter by ID/Pubkey..."
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                            className="pl-9 pr-4 py-2 rounded-xl bg-card border border-white/10 focus:border-primary/50 focus:ring-1 focus:ring-primary/50 transition-all text-sm w-64 shadow-inner px-2"
                        />
                    </div>
                    <button
                        onClick={() => {
                            setIsLoading(true);
                            fetchData();
                        }}
                        className="p-2.5 rounded-xl bg-card border border-white/10 hover:bg-white/5 transition-colors shadow-sm"
                    >
                        <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                    </button>
                </div>
            </div>

            <div className="flex items-center gap-2 p-1 bg-white/5 border border-white/5 rounded-xl w-fit mb-4">
                {(['all', 'instances', 'nodes'] as const).map((f) => (
                    <button
                        key={f}
                        onClick={() => setFilter(f)}
                        className={cn(
                            "px-4 py-1.5 rounded-lg text-[10px] font-bold uppercase tracking-widest transition-all",
                            filter === f ? "bg-primary text-primary-foreground shadow-md" : "text-muted-foreground hover:text-foreground"
                        )}
                    >
                        {f}
                    </button>
                ))}
            </div>

            {isLoading && !presence ? (
                <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                    {[1, 2, 3].map(i => (
                        <div key={i} className="h-44 rounded-2xl border border-white/5 bg-white/[0.02] animate-pulse" />
                    ))}
                </div>
            ) : filteredItems.length > 0 ? (
                <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                    <AnimatePresence mode="popLayout">
                        {filteredItems.map((item, idx) => (
                            <motion.div
                                key={item.id || item.pubkey || idx}
                                layout
                                initial={{ opacity: 0, y: 10 }}
                                animate={{ opacity: 1, y: 0 }}
                                exit={{ opacity: 0, scale: 0.95 }}
                            >
                                <PresenceItem type={item.p_type as any} data={item} />
                            </motion.div>
                        ))}
                    </AnimatePresence>
                </div>
            ) : (
                <div className="py-20 flex flex-col items-center justify-center text-center space-y-4">
                    <div className="p-4 rounded-full bg-white/5 border border-white/10">
                        <Users className="w-8 h-8 text-muted-foreground" />
                    </div>
                    <div>
                        <h3 className="text-lg font-semibold">No active presence found</h3>
                        <p className="text-sm text-muted-foreground">The mesh network is currently silent or your filter is too strict.</p>
                    </div>
                </div>
            )}

            <div className="p-6 rounded-2xl border bg-blue-500/5 border-blue-500/10 flex gap-4">
                <div className="p-2 bg-blue-500/10 rounded-xl h-fit">
                    <Info className="w-5 h-5 text-blue-400" />
                </div>
                <div className="space-y-1">
                    <h4 className="text-xs font-bold text-blue-400 uppercase tracking-widest">Mesh Network Dynamics</h4>
                    <p className="text-sm text-muted-foreground leading-relaxed">
                        Instances are local execution environments running within the gateway process.
                        Nodes represent external OpenClaw instances connected via the P2P mesh, allowing for task delegation and skill sharing.
                    </p>
                </div>
            </div>
        </motion.div>
    );
}
