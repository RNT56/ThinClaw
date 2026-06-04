import { useState, useRef, useCallback, useEffect } from 'react';

/**
 * Voice Activity Detection (VAD) hook.
 * Continuously monitors the microphone for speech using an AnalyserNode.
 * When speech energy exceeds the threshold for a sustained period, it
 * triggers a callback (e.g. to start recording).
 */

interface VoiceWakeOptions {
    /** RMS energy threshold (0–1). Default 0.03 */
    energyThreshold?: number;
    /** Milliseconds of sustained speech before triggering. Default 300ms */
    activationDelay?: number;
    /** Cooldown after trigger before re-listening (ms). Default 5000ms */
    cooldownMs?: number;
    /** Called when voice is detected */
    onWake: () => void;
}

export function useVoiceWake(options: VoiceWakeOptions) {
    const {
        energyThreshold = 0.03,
        activationDelay = 300,
        cooldownMs = 5000,
        onWake,
    } = options;

    const [isListening, setIsListening] = useState(false);
    const [currentEnergy, setCurrentEnergy] = useState(0);
    const audioContextRef = useRef<AudioContext | null>(null);
    const analyserRef = useRef<AnalyserNode | null>(null);
    const streamRef = useRef<MediaStream | null>(null);
    const rafRef = useRef<number>(0);
    const speechStartRef = useRef<number>(0);
    const cooldownRef = useRef<boolean>(false);
    const activeRef = useRef<boolean>(false);
    const onWakeRef = useRef(onWake);

    // Keep onWake ref current
    useEffect(() => {
        onWakeRef.current = onWake;
    }, [onWake]);

    const startListening = useCallback(async () => {
        if (activeRef.current) return;
        try {
            const stream = await navigator.mediaDevices.getUserMedia({
                audio: {
                    echoCancellation: true,
                    noiseSuppression: true,
                    autoGainControl: true,
                },
            });
            const audioContext = new AudioContext();
            const source = audioContext.createMediaStreamSource(stream);
            const analyser = audioContext.createAnalyser();
            analyser.fftSize = 512;
            analyser.smoothingTimeConstant = 0.8;
            source.connect(analyser);

            audioContextRef.current = audioContext;
            analyserRef.current = analyser;
            streamRef.current = stream;
            activeRef.current = true;
            setIsListening(true);

            // Start the monitoring loop
            const dataArray = new Float32Array(analyser.fftSize);

            const tick = () => {
                if (!activeRef.current) return;

                analyser.getFloatTimeDomainData(dataArray);

                // Calculate RMS energy
                let sum = 0;
                for (let i = 0; i < dataArray.length; i++) {
                    sum += dataArray[i] * dataArray[i];
                }
                const rms = Math.sqrt(sum / dataArray.length);
                setCurrentEnergy(rms);

                if (!cooldownRef.current) {
                    if (rms > energyThreshold) {
                        if (speechStartRef.current === 0) {
                            speechStartRef.current = Date.now();
                        } else if (Date.now() - speechStartRef.current > activationDelay) {
                            // Voice detected — trigger wake
                            cooldownRef.current = true;
                            speechStartRef.current = 0;
                            onWakeRef.current();
                            setTimeout(() => {
                                cooldownRef.current = false;
                            }, cooldownMs);
                        }
                    } else {
                        speechStartRef.current = 0;
                    }
                }

                rafRef.current = requestAnimationFrame(tick);
            };

            rafRef.current = requestAnimationFrame(tick);
        } catch (e) {
            console.error('Failed to start voice wake:', e);
            throw e;
        }
    }, [energyThreshold, activationDelay, cooldownMs]);

    const stopListening = useCallback(() => {
        activeRef.current = false;
        setIsListening(false);
        cancelAnimationFrame(rafRef.current);
        if (streamRef.current) {
            streamRef.current.getTracks().forEach(t => t.stop());
            streamRef.current = null;
        }
        if (audioContextRef.current) {
            audioContextRef.current.close();
            audioContextRef.current = null;
        }
        analyserRef.current = null;
        speechStartRef.current = 0;
        cooldownRef.current = false;
    }, []);

    // Cleanup on unmount
    useEffect(() => {
        return () => {
            if (activeRef.current) stopListening();
        };
    }, [stopListening]);

    return {
        isListening,
        currentEnergy,
        startListening,
        stopListening,
    };
}
