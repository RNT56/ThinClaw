import { useState, useRef, useCallback } from "react";

export function useAudioRecorder() {
    const [isRecording, setIsRecording] = useState(false);
    const mediaRecorder = useRef<MediaRecorder | null>(null);
    const chunks = useRef<Blob[]>([]);

    const startRecording = useCallback(async () => {
        try {
            const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
            // specific options to try and get compatible audio, though browser support varies
            // we default to default logic and depend on backend or browser to do right thing
            mediaRecorder.current = new MediaRecorder(stream);
            chunks.current = [];

            mediaRecorder.current.ondataavailable = (e) => {
                if (e.data.size > 0) chunks.current.push(e.data);
            };

            mediaRecorder.current.start(1000); // 1s timeslice for better reliability on long recordings
            setIsRecording(true);
        } catch (e) {
            console.error("Failed to start recording:", e);
            throw e;
        }
    }, []);

    const stopRecording = useCallback(async (): Promise<Blob> => {
        return new Promise((resolve) => {
            if (!mediaRecorder.current) {
                resolve(new Blob());
                return;
            }

            mediaRecorder.current.onstop = async () => {
                const webmBlob = new Blob(chunks.current, { type: 'audio/webm' });
                chunks.current = [];
                setIsRecording(false);
                mediaRecorder.current?.stream.getTracks().forEach(t => t.stop());

                try {
                    // Convert WebM to 16kHz WAV for Whisper
                    const audioContext = new (window.AudioContext || (window as any).webkitAudioContext)();
                    const arrayBuffer = await webmBlob.arrayBuffer();
                    const audioBuffer = await audioContext.decodeAudioData(arrayBuffer);

                    // Offline context for resampling
                    const offlineContext = new OfflineAudioContext(
                        1,
                        audioBuffer.duration * 16000,
                        16000
                    );
                    const source = offlineContext.createBufferSource();
                    source.buffer = audioBuffer;
                    source.connect(offlineContext.destination);
                    source.start();
                    const resampledBuffer = await offlineContext.startRendering();

                    // Encode to WAV
                    const wavBlob = bufferToWav(resampledBuffer);
                    resolve(wavBlob);
                } catch (e) {
                    console.error("Audio conversion failed:", e);
                    // Fallback to original blob if conversion fails (though it likely won't work with Whisper)
                    resolve(webmBlob);
                }
            };

            mediaRecorder.current.stop();
        });
    }, []);

    // Helper to encode AudioBuffer to WAV
    function bufferToWav(abuffer: AudioBuffer) {
        const numOfChan = abuffer.numberOfChannels;
        const length = abuffer.length * numOfChan * 2 + 44;
        const buffer = new ArrayBuffer(length);
        const view = new DataView(buffer);
        const channels = [];
        let i;
        let sample;
        let offset = 0;
        let pos = 0;

        // write WAVE header
        setUint32(0x46464952); // "RIFF"
        setUint32(length - 8); // file length - 8
        setUint32(0x45564157); // "WAVE"

        setUint32(0x20746d66); // "fmt " chunk
        setUint32(16); // length = 16
        setUint16(1); // PCM (uncompressed)
        setUint16(numOfChan);
        setUint32(abuffer.sampleRate);
        setUint32(abuffer.sampleRate * 2 * numOfChan); // avg. bytes/sec
        setUint16(numOfChan * 2); // block-align
        setUint16(16); // 16-bit (hardcoded in this example)

        setUint32(0x61746164); // "data" - chunk
        setUint32(length - pos - 4); // chunk length

        // write interleaved data
        for (i = 0; i < abuffer.numberOfChannels; i++)
            channels.push(abuffer.getChannelData(i));

        while (pos < abuffer.length) {
            for (i = 0; i < numOfChan; i++) {
                // interleave channels
                sample = Math.max(-1, Math.min(1, channels[i][pos])); // clamp
                sample = (0.5 + sample < 0 ? sample * 32768 : sample * 32767) | 0; // scale to 16-bit signed int
                view.setInt16(44 + offset, sample, true); // write 16-bit sample
                offset += 2;
            }
            pos++;
        }

        return new Blob([buffer], { type: "audio/wav" });

        function setUint16(data: any) {
            view.setUint16(pos, data, true);
            pos += 2;
        }

        function setUint32(data: any) {
            view.setUint32(pos, data, true);
            pos += 4;
        }
    }

    return { isRecording, startRecording, stopRecording };
}
