export const VISION_KEYWORDS = [
    "pixtral",
    "llava",
    "vision",
    "gemma",
    "clip",
    "moondream",
    "qwen-vl",
    "qwen3-vl",
    "lfm",
    "liquid",
    "bakllava",
    "yi-vl",
    "glm-4",
    "ministral",
];

export interface LocalVisionMetadata {
    source?: string | null;
    task?: string | null;
    companion_path?: string | null;
    compatible?: boolean;
}

export function isVisionCapable(
    modelPath: string,
    inventoryModel?: LocalVisionMetadata,
): boolean {
    // Managed installs have an authoritative task contract. Do not let a
    // filename such as "gemma" turn a managed chat-only artifact into a vision
    // model, or hide an explicitly managed vision model with an unusual name.
    if (inventoryModel && inventoryModel.source !== "legacy") {
        return inventoryModel.compatible !== false
            && (
                inventoryModel.task === "vision"
                || Boolean(inventoryModel.companion_path)
            );
    }
    if (!modelPath) return false;
    const lower = modelPath.toLowerCase();
    return VISION_KEYWORDS.some(keyword => lower.includes(keyword));
}
