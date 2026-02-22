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
    "ministral"
];

export function isVisionCapable(modelPath: string): boolean {
    if (!modelPath) return false;
    const lower = modelPath.toLowerCase();
    return VISION_KEYWORDS.some(keyword => lower.includes(keyword));
}
