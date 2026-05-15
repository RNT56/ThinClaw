/**
 * Memory analysis utilities for predicting model RAM usage
 * and checking against system constraints.
 */
import { GGUFMetadata } from '../../lib/bindings';

// 1 GB = 1024^3 bytes
export const GB = 1073741824;

export interface MemoryAnalysis {
    canRun: boolean;
    risk: "Safe" | "Moderate" | "Critical";
    totalNeededRef: number; // in GB
    details: string;
    predictedTokensPerSec: number;
}

export function analyzeMemoryConstraints(
    ctx: number,
    totalRamBytes: number,
    modelSizeBytes: number,
    reservationGb: number,
    enableReservation: boolean,
    usedMemoryBytes: number,
    appMemoryBytes: number,
    quantizeKv: boolean,
    bandwidthGbps: number,
    metadata?: GGUFMetadata
): MemoryAnalysis {
    let availableForAI = 0;
    let limitLabel = "";

    // Calculate physical headroom: total RAM minus what the OS and other apps are using (excluding our app's AI usage)
    const physicalHeadroomForAI = totalRamBytes - (usedMemoryBytes - appMemoryBytes);

    if (enableReservation) {
        const quotaBytes = reservationGb * GB;
        // The effective limit is the MIN of user quota and actual physical headroom
        availableForAI = Math.min(quotaBytes, physicalHeadroomForAI);

        limitLabel = availableForAI < quotaBytes ? "Physical Limit (System Full)" : `${reservationGb}GB AI Quota`;
    } else {
        availableForAI = Math.max(0, physicalHeadroomForAI);
        limitLabel = "Physical Headroom";
    }

    // GGUF models are already quantized, so weightLoad is basically the file size
    const weightLoad = modelSizeBytes * 1.05;

    let kvLoad = 0;
    let breakdown = "";

    if (metadata && metadata.block_count > 0) {
        // KV size per token = 2 * layers * heads_kv * head_dim * precision
        const n_layers = metadata.block_count;
        const n_heads = metadata.head_count;
        const n_heads_kv = metadata.head_count_kv;
        const n_embd = metadata.embedding_length;
        const head_dim = n_heads > 0 ? n_embd / n_heads : 128;

        // llama.cpp default KV is F16 (2 bytes)
        // If quantizeKv is true, we use Q4_0 (0.5 bytes approx / 4-bit)
        const bytes_per_element = quantizeKv ? 0.5 : 2.0;
        const bytes_per_token = 2 * n_layers * n_heads_kv * head_dim * bytes_per_element;
        kvLoad = ctx * bytes_per_token;

        breakdown = `${metadata.architecture.toUpperCase()} ${n_layers}L. `;
    } else {
        const baseKv = 204800;
        kvLoad = ctx * (quantizeKv ? baseKv * 0.25 : baseKv);
        breakdown = "Estimate: ";
    }

    const totalNeeded = weightLoad + kvLoad;
    const totalNeededGB = totalNeeded / GB;

    let risk: "Safe" | "Moderate" | "Critical" = "Safe";
    if (totalNeeded > availableForAI * 0.9) risk = "Critical";
    else if (totalNeeded > availableForAI * 0.7) risk = "Moderate";

    // Speed (tok/s) = Bandwidth / (Model weights + KV Cache)
    // We add a 20% penalty for system overhead and 4-bit KV boost if active
    const kvBoost = quantizeKv ? 1.05 : 1.0; // Minimal scaling for decoding
    const totalDataToRead = weightLoad + kvLoad;
    const predictedTokensPerSec = (bandwidthGbps / (totalDataToRead / GB)) * 0.85 * kvBoost;

    return {
        canRun: totalNeeded <= availableForAI,
        risk,
        totalNeededRef: totalNeededGB,
        predictedTokensPerSec,
        details: `${breakdown}Weights: ${(weightLoad / GB).toFixed(1)}GB + Cache: ${(kvLoad / GB).toFixed(2)}GB ≈ ${totalNeededGB.toFixed(1)}GB. Limit: ${(availableForAI / GB).toFixed(1)}GB (${limitLabel}).`
    };
}
