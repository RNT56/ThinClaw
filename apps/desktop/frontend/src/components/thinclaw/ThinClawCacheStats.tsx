import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import { Database, RefreshCw, Zap, TrendingDown, HardDrive, Percent } from 'lucide-react';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';

function StatCard({ icon: Icon, label, value, sub, color }: {
    icon: any; label: string; value: string; sub?: string; color: string;
}) {
    return (
        <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
            <div className="flex items-center gap-2 mb-3">
                <Icon className={cn("w-4 h-4", color)} />
                <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">{label}</span>
            </div>
            <p className={cn("text-3xl font-bold tabular-nums", color)}>{value}</p>
            {sub && <p className="text-[10px] text-muted-foreground mt-1">{sub}</p>}
        </div>
    );
}

export function ThinClawCacheStats() {
    const [stats, setStats] = useState<thinclaw.CacheStats | null>(null);
    const [isLoading, setIsLoading] = useState(true);

    const fetchData = useCallback(async () => {
        try {
            const data = await thinclaw.getCacheStats();
            setStats(data);
        } catch (e) {
            console.error('Failed to fetch cache stats:', e);
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 30000);
        return () => clearInterval(interval);
    }, [fetchData]);

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center">
                <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
            </div>
        );
    }

    const hitRate = stats?.hit_rate ?? 0;
    const sizeKB = ((stats?.size_bytes ?? 0) / 1024).toFixed(1);
    const sizeMB = ((stats?.size_bytes ?? 0) / (1024 * 1024)).toFixed(2);
    const displaySize = (stats?.size_bytes ?? 0) > 1024 * 1024 ? `${sizeMB} MB` : `${sizeKB} KB`;

    return (
        <motion.div
            className="flex-1 overflow-y-auto p-8 space-y-8"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-cyan-500/10 border border-cyan-500/20">
                        <Database className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Response Cache</h1>
                        <p className="text-xs text-muted-foreground">
                            Cache statistics for LLM response deduplication
                        </p>
                    </div>
                </div>
                <button
                    onClick={fetchData}
                    className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                >
                    <RefreshCw className={cn("w-3.5 h-3.5", isLoading && "animate-spin")} />
                </button>
            </div>

            {/* Stat cards */}
            <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
                <StatCard
                    icon={Zap}
                    label="Hits"
                    value={(stats?.hits ?? 0).toLocaleString()}
                    sub="Served from cache"
                    color="text-primary"
                />
                <StatCard
                    icon={TrendingDown}
                    label="Misses"
                    value={(stats?.misses ?? 0).toLocaleString()}
                    sub="Required LLM call"
                    color="text-muted-foreground"
                />
                <StatCard
                    icon={Percent}
                    label="Hit Rate"
                    value={`${(hitRate * 100).toFixed(1)}%`}
                    sub={hitRate >= 0.7 ? 'Excellent' : hitRate >= 0.4 ? 'Good' : 'Low'}
                    color={hitRate >= 0.7 ? 'text-primary' : hitRate >= 0.4 ? 'text-muted-foreground' : 'text-red-400'}
                />
                <StatCard
                    icon={HardDrive}
                    label="Size"
                    value={displaySize}
                    sub={`${(stats?.evictions ?? 0).toLocaleString()} evictions`}
                    color="text-blue-400"
                />
            </div>

            {/* Hit rate visualisation */}
            <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-6">
                <h3 className="text-sm font-bold text-muted-foreground mb-4">Cache Efficiency</h3>
                <div className="h-6 bg-white/[0.03] rounded-full overflow-hidden border border-white/5 relative">
                    <motion.div
                        initial={{ width: 0 }}
                        animate={{ width: `${hitRate * 100}%` }}
                        transition={{ duration: 0.8, ease: 'easeOut' }}
                        className="h-full rounded-full bg-gradient-to-r from-emerald-500/80 to-cyan-500/80"
                    />
                    <span className="absolute inset-0 flex items-center justify-center text-[10px] font-bold text-white/70">
                        {(hitRate * 100).toFixed(1)}% hit rate
                    </span>
                </div>
                <div className="flex justify-between mt-2">
                    <span className="text-[9px] text-muted-foreground">0%</span>
                    <span className="text-[9px] text-muted-foreground">100%</span>
                </div>
            </div>
        </motion.div>
    );
}
