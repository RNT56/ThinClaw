export type ImageGenerationProvider =
    | "local"
    | "nano-banana"
    | "nano-banana-pro";

/** Validate local-model availability before prompt enhancement or generation. */
export function requireLocalImageModelPath(
    provider: ImageGenerationProvider,
    resolvedModelPath: string | null | undefined,
): string | undefined {
    if (provider !== "local") return undefined;
    if (!resolvedModelPath) {
        throw new Error(
            "No compatible local image generation model is available. "
            + "Download a diffusion model in Models → Discover.",
        );
    }
    return resolvedModelPath;
}

/** Start the local image runtime through the rejecting command client. */
export async function startLocalImageRuntime({
    modelPath,
    start,
}: {
    modelPath: string;
    start: (modelPath: string) => Promise<null>;
}): Promise<void> {
    await start(modelPath);
}
