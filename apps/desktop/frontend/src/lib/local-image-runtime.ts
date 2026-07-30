import { unwrapResult } from "./guards";

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

/** Start the local image runtime and convert backend Result errors to failures. */
export async function startLocalImageRuntime<E>({
    modelPath,
    start,
}: {
    modelPath: string;
    start: (
        modelPath: string,
    ) => Promise<{ status: "ok"; data: null } | { status: "error"; error: E }>;
}): Promise<void> {
    unwrapResult(
        await start(modelPath),
        "start local image generation runtime",
    );
}
