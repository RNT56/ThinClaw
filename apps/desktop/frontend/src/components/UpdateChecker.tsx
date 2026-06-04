import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Download, RefreshCw, Loader2, CheckCircle2, X, Sparkles } from 'lucide-react';
import { check, Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

type UpdateStatus = 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'error' | 'upToDate';

interface DownloadProgress {
    downloaded: number;
    total: number;
}

export function UpdateChecker() {
    const [status, setStatus] = useState<UpdateStatus>('idle');
    const [update, setUpdate] = useState<Update | null>(null);
    const [progress, setProgress] = useState<DownloadProgress | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [dismissed, setDismissed] = useState(false);

    const checkForUpdates = useCallback(async () => {
        setStatus('checking');
        setError(null);
        try {
            const result = await check();
            if (result) {
                setUpdate(result);
                setStatus('available');
                setDismissed(false);
            } else {
                setStatus('upToDate');
                setTimeout(() => setStatus('idle'), 3000);
            }
        } catch (e) {
            setError(String(e));
            setStatus('error');
            setTimeout(() => setStatus('idle'), 5000);
        }
    }, []);

    // Check on mount (with a short delay to not block startup)
    useEffect(() => {
        const timer = setTimeout(checkForUpdates, 5000);
        return () => clearTimeout(timer);
    }, [checkForUpdates]);

    const handleDownloadAndInstall = async () => {
        if (!update) return;
        setStatus('downloading');
        try {
            await update.downloadAndInstall((event) => {
                switch (event.event) {
                    case 'Started':
                        setProgress({ downloaded: 0, total: event.data.contentLength ?? 0 });
                        break;
                    case 'Progress':
                        setProgress(prev => ({
                            downloaded: (prev?.downloaded ?? 0) + event.data.chunkLength,
                            total: prev?.total ?? 0,
                        }));
                        break;
                    case 'Finished':
                        setProgress(null);
                        break;
                }
            });
            setStatus('ready');
        } catch (e) {
            setError(String(e));
            setStatus('error');
        }
    };

    const handleRelaunch = async () => {
        await relaunch();
    };

    // Don't render anything in idle/checking states (unless actively checking)
    if (status === 'idle') return null;
    if (dismissed && status === 'available') return null;

    return (
        <AnimatePresence>
            <motion.div
                initial={{ opacity: 0, y: -20, scale: 0.95 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -20, scale: 0.95 }}
                style={{
                    position: 'fixed',
                    top: 12,
                    right: 12,
                    zIndex: 9999,
                    width: 340,
                }}
            >
                <div style={{
                    background: 'linear-gradient(135deg, rgba(13,17,23,0.95), rgba(22,27,34,0.95))',
                    backdropFilter: 'blur(20px)',
                    borderRadius: 14,
                    border: '1px solid rgba(255,255,255,0.08)',
                    boxShadow: '0 8px 32px rgba(0,0,0,0.4), 0 0 0 1px rgba(255,255,255,0.05)',
                    overflow: 'hidden',
                }}>
                    {/* Header */}
                    <div style={{
                        padding: '14px 16px 10px',
                        display: 'flex',
                        alignItems: 'center',
                        gap: 10,
                    }}>
                        <div style={{
                            width: 32,
                            height: 32,
                            borderRadius: 8,
                            background: status === 'available' || status === 'ready'
                                ? 'linear-gradient(135deg, rgba(56,139,253,0.2), rgba(139,92,246,0.2))'
                                : status === 'error'
                                    ? 'rgba(248,81,73,0.15)'
                                    : 'rgba(255,255,255,0.06)',
                            display: 'flex',
                            alignItems: 'center',
                            justifyContent: 'center',
                        }}>
                            {status === 'checking' ? (
                                <Loader2 size={16} color="#8b949e" className="animate-spin" />
                            ) : status === 'available' ? (
                                <Sparkles size={16} color="#58a6ff" />
                            ) : status === 'downloading' ? (
                                <Download size={16} color="#58a6ff" />
                            ) : status === 'ready' ? (
                                <CheckCircle2 size={16} color="#3fb950" />
                            ) : status === 'upToDate' ? (
                                <CheckCircle2 size={16} color="#3fb950" />
                            ) : status === 'error' ? (
                                <X size={16} color="#f85149" />
                            ) : null}
                        </div>

                        <div style={{ flex: 1 }}>
                            <div style={{
                                fontSize: 13,
                                fontWeight: 600,
                                color: '#e6edf3',
                            }}>
                                {status === 'checking' ? 'Checking for updates...'
                                    : status === 'available' ? `Update ${update?.version} available`
                                        : status === 'downloading' ? 'Downloading update...'
                                            : status === 'ready' ? 'Ready to restart'
                                                : status === 'upToDate' ? 'Up to date'
                                                    : status === 'error' ? 'Update failed'
                                                        : ''}
                            </div>
                            {update?.body && status === 'available' && (
                                <div style={{
                                    fontSize: 11,
                                    color: '#8b949e',
                                    marginTop: 2,
                                    lineHeight: '1.4',
                                    maxHeight: 40,
                                    overflow: 'hidden',
                                }}>
                                    {update.body}
                                </div>
                            )}
                        </div>

                        {(status === 'available' || status === 'error' || status === 'upToDate') && (
                            <button
                                onClick={() => {
                                    if (status === 'available') setDismissed(true);
                                    else setStatus('idle');
                                }}
                                style={{
                                    background: 'none',
                                    border: 'none',
                                    cursor: 'pointer',
                                    padding: 4,
                                    color: '#484f58',
                                }}
                            >
                                <X size={14} />
                            </button>
                        )}
                    </div>

                    {/* Progress */}
                    {status === 'downloading' && progress && (
                        <div style={{ padding: '0 16px 10px' }}>
                            <div style={{
                                height: 3,
                                borderRadius: 2,
                                background: 'rgba(255,255,255,0.06)',
                                overflow: 'hidden',
                            }}>
                                <motion.div
                                    style={{
                                        height: '100%',
                                        background: 'linear-gradient(90deg, #388bfd, #8b5cf6)',
                                        borderRadius: 2,
                                    }}
                                    initial={{ width: 0 }}
                                    animate={{
                                        width: progress.total > 0
                                            ? `${(progress.downloaded / progress.total) * 100}%`
                                            : '50%',
                                    }}
                                    transition={{ duration: 0.3 }}
                                />
                            </div>
                            {progress.total > 0 && (
                                <div style={{
                                    fontSize: 10,
                                    color: '#484f58',
                                    marginTop: 4,
                                    fontFamily: 'monospace',
                                }}>
                                    {(progress.downloaded / 1024 / 1024).toFixed(1)} MB / {(progress.total / 1024 / 1024).toFixed(1)} MB
                                </div>
                            )}
                        </div>
                    )}

                    {/* Actions */}
                    {(status === 'available' || status === 'ready') && (
                        <div style={{
                            padding: '0 16px 14px',
                            display: 'flex',
                            gap: 8,
                        }}>
                            {status === 'available' ? (
                                <button
                                    onClick={handleDownloadAndInstall}
                                    style={{
                                        flex: 1,
                                        padding: '8px 0',
                                        borderRadius: 8,
                                        border: 'none',
                                        background: 'linear-gradient(135deg, #388bfd, #8b5cf6)',
                                        color: '#fff',
                                        fontSize: 12,
                                        fontWeight: 600,
                                        cursor: 'pointer',
                                        display: 'flex',
                                        alignItems: 'center',
                                        justifyContent: 'center',
                                        gap: 6,
                                    }}
                                >
                                    <Download size={13} />
                                    Download & Install
                                </button>
                            ) : (
                                <button
                                    onClick={handleRelaunch}
                                    style={{
                                        flex: 1,
                                        padding: '8px 0',
                                        borderRadius: 8,
                                        border: 'none',
                                        background: 'linear-gradient(135deg, #238636, #2ea043)',
                                        color: '#fff',
                                        fontSize: 12,
                                        fontWeight: 600,
                                        cursor: 'pointer',
                                        display: 'flex',
                                        alignItems: 'center',
                                        justifyContent: 'center',
                                        gap: 6,
                                    }}
                                >
                                    <RefreshCw size={13} />
                                    Restart Now
                                </button>
                            )}
                        </div>
                    )}

                    {/* Error details */}
                    {status === 'error' && error && (
                        <div style={{
                            padding: '0 16px 14px',
                            fontSize: 10,
                            color: '#f85149',
                            fontFamily: 'monospace',
                            lineHeight: '1.4',
                            maxHeight: 60,
                            overflow: 'hidden',
                        }}>
                            {error}
                        </div>
                    )}
                </div>
            </motion.div>
        </AnimatePresence>
    );
}
