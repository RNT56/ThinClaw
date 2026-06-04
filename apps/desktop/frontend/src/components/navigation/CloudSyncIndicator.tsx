/**
 * A5-9: CloudSyncIndicator — tiny pill that shows current cloud storage status.
 * Renders inline in the sidebar, just above the Settings button.
 * Shows: connected/local mode, a pulsing dot when migrating, and migration progress.
 */
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Cloud, Loader2 } from 'lucide-react';
import { cn } from '../../lib/utils';
import { motion, AnimatePresence } from 'framer-motion';

interface CloudSyncState {
    mode: string;
    migrating: boolean;
    overallPercent: number;
}

export function CloudSyncIndicator({ sidebarOpen }: { sidebarOpen: boolean }) {
    const [state, setState] = useState<CloudSyncState>({
        mode: 'local',
        migrating: false,
        overallPercent: 0,
    });

    // Poll cloud status
    useEffect(() => {
        let mounted = true;
        const poll = async () => {
            try {
                const s = await invoke<{
                    mode: string;
                    migration_in_progress: boolean;
                }>('cloud_get_status');
                if (mounted) {
                    setState(prev => ({
                        ...prev,
                        mode: s.mode,
                        migrating: s.migration_in_progress,
                    }));
                }
            } catch {
                // Cloud manager not initialized yet — ignore
            }
        };
        poll();
        const interval = setInterval(poll, 15_000);
        return () => { mounted = false; clearInterval(interval); };
    }, []);

    // Listen for migration progress
    useEffect(() => {
        let unlisten: UnlistenFn | null = null;
        (async () => {
            unlisten = await listen<{
                overall_percent: number;
                complete: boolean;
                error: string | null;
            }>('cloud_migration_progress', event => {
                const p = event.payload;
                setState(prev => ({
                    ...prev,
                    migrating: !p.complete && !p.error,
                    overallPercent: p.overall_percent,
                }));
            });
        })();
        return () => { unlisten?.(); };
    }, []);

    const isCloud = state.mode.startsWith('cloud:');

    // Don't show anything if local and not migrating — keep it clean
    if (!isCloud && !state.migrating) return null;

    return (
        <AnimatePresence>
            <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: 'auto' }}
                exit={{ opacity: 0, height: 0 }}
                className="overflow-hidden"
            >
                <div className={cn(
                    'flex items-center gap-2 rounded-lg transition-all duration-300',
                    sidebarOpen ? 'px-3 py-1.5' : 'justify-center py-1.5',
                    state.migrating
                        ? 'bg-blue-500/10 text-blue-500'
                        : 'text-muted-foreground/60'
                )}>
                    {state.migrating ? (
                        <>
                            <Loader2 className="w-3 h-3 animate-spin shrink-0" />
                            <AnimatePresence>
                                {sidebarOpen && (
                                    <motion.div
                                        initial={{ width: 0, opacity: 0 }}
                                        animate={{ width: 'auto', opacity: 1 }}
                                        exit={{ width: 0, opacity: 0 }}
                                        className="overflow-hidden"
                                    >
                                        <span className="text-[10px] font-bold uppercase tracking-wider whitespace-nowrap">
                                            Syncing {state.overallPercent.toFixed(0)}%
                                        </span>
                                    </motion.div>
                                )}
                            </AnimatePresence>
                        </>
                    ) : (
                        <>
                            <div className="relative">
                                <Cloud className="w-3 h-3 shrink-0" />
                                <div className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-emerald-500 border border-background" />
                            </div>
                            <AnimatePresence>
                                {sidebarOpen && (
                                    <motion.div
                                        initial={{ width: 0, opacity: 0 }}
                                        animate={{ width: 'auto', opacity: 1 }}
                                        exit={{ width: 0, opacity: 0 }}
                                        className="overflow-hidden"
                                    >
                                        <span className="text-[10px] font-medium whitespace-nowrap">
                                            Cloud Synced
                                        </span>
                                    </motion.div>
                                )}
                            </AnimatePresence>
                        </>
                    )}
                </div>
            </motion.div>
        </AnimatePresence>
    );
}
