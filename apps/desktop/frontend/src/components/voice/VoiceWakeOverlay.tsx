import { useState, useCallback, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Mic, MicOff, Volume2, Loader2, Waves } from 'lucide-react';
import { useVoiceWake } from '../../hooks/use-voice-wake';
import { useAudioRecorder } from '../../hooks/use-audio-recorder';
import { directCommands } from '../../lib/generated/direct-commands';
import { toast } from 'sonner';

type VoiceState = 'idle' | 'listening' | 'recording' | 'transcribing';

export function VoiceWakeOverlay() {
    const [enabled, setEnabled] = useState(() => {
        try { return localStorage.getItem('voice_wake_enabled') === 'true'; }
        catch (_) { return false; }
    });
    const [voiceState, setVoiceState] = useState<VoiceState>('idle');
    const [showOverlay, setShowOverlay] = useState(false);
    const showOverlayTimerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
    const { startRecording, stopRecording } = useAudioRecorder();
    const recordingTimeoutRef = useRef<ReturnType<typeof setTimeout>>(undefined);

    // Save preference
    useEffect(() => {
        localStorage.setItem('voice_wake_enabled', String(enabled));
    }, [enabled]);

    const handleStopAndTranscribe = useCallback(async () => {
        if (recordingTimeoutRef.current) clearTimeout(recordingTimeoutRef.current);

        setVoiceState('transcribing');
        try {
            const blob = await stopRecording();
            if (blob.size === 0) {
                setVoiceState('listening');
                setShowOverlay(false);
                return;
            }

            const arrayBuffer = await blob.arrayBuffer();
            const audioBytes = Array.from(new Uint8Array(arrayBuffer));

            const res = await directCommands.directMediaTranscribeAudio(audioBytes);

            if (res.status === 'error') {
                toast.error('Transcription failed', { description: res.error });
            } else if (res.status === 'ok' && res.data.text.trim()) {
                const text = res.data.text.trim();
                // Dispatch custom event for ChatProvider to pick up
                window.dispatchEvent(
                    new CustomEvent('voice-wake-transcription', { detail: text.trim() })
                );
                toast.success('Voice input received', {
                    description: text.slice(0, 80) + (text.length > 80 ? '...' : ''),
                });
            }
        } catch (e) {
            console.error('Transcription error:', e);
            toast.error('Voice transcription failed');
        } finally {
            setVoiceState('listening');
            showOverlayTimerRef.current = setTimeout(() => setShowOverlay(false), 1500);
        }
    }, [stopRecording]);

    const handleWake = useCallback(async () => {
        if (voiceState !== 'listening') return;

        setShowOverlay(true);
        setVoiceState('recording');

        try {
            await startRecording();

            // Auto-stop recording after 30 seconds max
            recordingTimeoutRef.current = setTimeout(async () => {
                await handleStopAndTranscribe();
            }, 30000);
        } catch (e) {
            console.error('Failed to start recording after wake:', e);
            setVoiceState('listening');
            setShowOverlay(false);
        }
    }, [voiceState, startRecording, handleStopAndTranscribe]);

    const { isListening, currentEnergy, startListening, stopListening } = useVoiceWake({
        energyThreshold: 0.035,
        activationDelay: 400,
        cooldownMs: 3000,
        onWake: handleWake,
    });

    const handleToggle = useCallback(async () => {
        if (enabled) {
            // Turning off
            stopListening();
            setEnabled(false);
            setVoiceState('idle');
            setShowOverlay(false);
        } else {
            // Turning on
            setEnabled(true);
            try {
                await startListening();
                setVoiceState('listening');
                toast.success('Voice wake enabled', { description: 'Speak to activate recording' });
            } catch (e) {
                setEnabled(false);
                toast.error('Microphone access denied');
            }
        }
    }, [enabled, startListening, stopListening]);

    // Auto-start if enabled on mount
    useEffect(() => {
        if (enabled && !isListening) {
            startListening()
                .then(() => setVoiceState('listening'))
                .catch(() => setEnabled(false));
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // Cleanup
    useEffect(() => {
        return () => {
            if (recordingTimeoutRef.current) clearTimeout(recordingTimeoutRef.current);
            if (showOverlayTimerRef.current) clearTimeout(showOverlayTimerRef.current);
        };
    }, []);

    // Energy visualization (0-1 mapped to visual width)
    const energyPercent = Math.min(currentEnergy * 15, 1);

    return (
        <>
            {/* Floating toggle button */}
            <motion.button
                onClick={handleToggle}
                whileHover={{ scale: 1.05 }}
                whileTap={{ scale: 0.95 }}
                style={{
                    position: 'fixed',
                    bottom: 20,
                    left: 20,
                    zIndex: 9998,
                    width: 44,
                    height: 44,
                    borderRadius: 22,
                    border: 'none',
                    cursor: 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    background: enabled
                        ? voiceState === 'recording'
                            ? 'linear-gradient(135deg, #ef4444, #dc2626)'
                            : 'linear-gradient(135deg, #22c55e, #16a34a)'
                        : 'rgba(255,255,255,0.06)',
                    boxShadow: enabled
                        ? voiceState === 'recording'
                            ? '0 0 20px rgba(239,68,68,0.4)'
                            : '0 0 12px rgba(34,197,94,0.3)'
                        : '0 2px 8px rgba(0,0,0,0.3)',
                    transition: 'background 0.3s, box-shadow 0.3s',
                }}
                title={enabled ? 'Disable voice wake' : 'Enable voice wake'}
            >
                {enabled ? (
                    voiceState === 'recording' ? (
                        <Mic size={18} color="#fff" />
                    ) : voiceState === 'transcribing' ? (
                        <Loader2 size={18} color="#fff" className="animate-spin" />
                    ) : (
                        <Volume2 size={18} color="#fff" />
                    )
                ) : (
                    <MicOff size={18} color="#6b7280" />
                )}

                {/* Energy ring when listening */}
                {enabled && voiceState === 'listening' && (
                    <motion.div
                        style={{
                            position: 'absolute',
                            inset: -3,
                            borderRadius: '50%',
                            border: '2px solid rgba(34,197,94,0.3)',
                            opacity: energyPercent,
                        }}
                        animate={{ scale: 1 + energyPercent * 0.3 }}
                        transition={{ duration: 0.1 }}
                    />
                )}

                {/* Pulsing ring when recording */}
                {voiceState === 'recording' && (
                    <motion.div
                        style={{
                            position: 'absolute',
                            inset: -6,
                            borderRadius: '50%',
                            border: '2px solid rgba(239,68,68,0.5)',
                        }}
                        animate={{
                            scale: [1, 1.3, 1],
                            opacity: [0.5, 0.2, 0.5],
                        }}
                        transition={{
                            duration: 1.5,
                            repeat: Infinity,
                        }}
                    />
                )}
            </motion.button>

            {/* Recording overlay */}
            <AnimatePresence>
                {showOverlay && (
                    <motion.div
                        initial={{ opacity: 0, y: 30, scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 20, scale: 0.95 }}
                        style={{
                            position: 'fixed',
                            bottom: 76,
                            left: 20,
                            zIndex: 9997,
                            width: 280,
                        }}
                    >
                        <div style={{
                            background: 'linear-gradient(135deg, rgba(13,17,23,0.96), rgba(22,27,34,0.96))',
                            backdropFilter: 'blur(20px)',
                            borderRadius: 14,
                            border: `1px solid ${voiceState === 'recording'
                                ? 'rgba(239,68,68,0.3)'
                                : voiceState === 'transcribing'
                                    ? 'rgba(139,92,246,0.3)'
                                    : 'rgba(255,255,255,0.08)'}`,
                            boxShadow: '0 8px 32px rgba(0,0,0,0.5)',
                            padding: '14px 16px',
                        }}>
                            <div style={{
                                display: 'flex',
                                alignItems: 'center',
                                gap: 10,
                            }}>
                                <div style={{
                                    width: 32,
                                    height: 32,
                                    borderRadius: 8,
                                    background: voiceState === 'recording'
                                        ? 'rgba(239,68,68,0.15)'
                                        : voiceState === 'transcribing'
                                            ? 'rgba(139,92,246,0.15)'
                                            : 'rgba(34,197,94,0.15)',
                                    display: 'flex',
                                    alignItems: 'center',
                                    justifyContent: 'center',
                                }}>
                                    {voiceState === 'recording' ? (
                                        <Waves size={16} color="#ef4444" />
                                    ) : voiceState === 'transcribing' ? (
                                        <Loader2 size={16} color="#8b5cf6" className="animate-spin" />
                                    ) : (
                                        <Volume2 size={16} color="#22c55e" />
                                    )}
                                </div>
                                <div>
                                    <div style={{
                                        fontSize: 13,
                                        fontWeight: 600,
                                        color: '#e6edf3',
                                    }}>
                                        {voiceState === 'recording'
                                            ? 'Recording...'
                                            : voiceState === 'transcribing'
                                                ? 'Transcribing...'
                                                : 'Listening'}
                                    </div>
                                    <div style={{
                                        fontSize: 11,
                                        color: '#8b949e',
                                        marginTop: 1,
                                    }}>
                                        {voiceState === 'recording'
                                            ? 'Click the mic or press Stop'
                                            : voiceState === 'transcribing'
                                                ? 'Processing your speech...'
                                                : 'Say something to activate'}
                                    </div>
                                </div>
                            </div>

                            {/* Audio level bar when recording */}
                            {voiceState === 'recording' && (
                                <div style={{
                                    marginTop: 10,
                                    height: 3,
                                    borderRadius: 2,
                                    background: 'rgba(255,255,255,0.06)',
                                    overflow: 'hidden',
                                }}>
                                    <motion.div
                                        style={{
                                            height: '100%',
                                            background: 'linear-gradient(90deg, #ef4444, #f97316)',
                                            borderRadius: 2,
                                        }}
                                        animate={{
                                            width: `${Math.max(10, energyPercent * 100)}%`,
                                        }}
                                        transition={{ duration: 0.05 }}
                                    />
                                </div>
                            )}

                            {/* Stop button when recording */}
                            {voiceState === 'recording' && (
                                <button
                                    onClick={handleStopAndTranscribe}
                                    style={{
                                        marginTop: 10,
                                        width: '100%',
                                        padding: '7px 0',
                                        borderRadius: 8,
                                        border: 'none',
                                        background: 'rgba(239,68,68,0.15)',
                                        color: '#ef4444',
                                        fontSize: 12,
                                        fontWeight: 600,
                                        cursor: 'pointer',
                                        display: 'flex',
                                        alignItems: 'center',
                                        justifyContent: 'center',
                                        gap: 6,
                                    }}
                                >
                                    <MicOff size={13} />
                                    Stop & Transcribe
                                </button>
                            )}
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </>
    );
}
