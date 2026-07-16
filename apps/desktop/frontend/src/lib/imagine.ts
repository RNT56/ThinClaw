import { commandClient } from './command-client';

// Re-export findStyle for convenience
export { findStyle } from './style-library';

export interface ImagineParams {
    prompt: string;
    provider: 'local' | 'nano-banana' | 'nano-banana-pro';
    aspectRatio: string;
    resolution?: string;
    styleId?: string;
    stylePrompt?: string;
    sourceImages?: string[];
    model?: string;
    steps?: number;
}

export interface GeneratedImage {
    id: string;
    prompt: string;
    styleId: string | null;
    provider: string;
    aspectRatio: string;
    resolution: string | null;
    width: number | null;
    height: number | null;
    seed: number | null;
    filePath: string;
    thumbnailPath: string | null;
    createdAt: string;
    isFavorite: boolean;
    tags: string | null;
}

export interface ImagineStats {
    total: number;
    favorites: number;
    byProvider: Array<{ provider: string; count: number }>;
}

/**
 * Generate an image using the Imagine mode
 */
export async function directImagineGenerate(params: ImagineParams): Promise<GeneratedImage> {
    return commandClient.directImagineGenerate({
        prompt: params.prompt,
        provider: params.provider,
        aspect_ratio: params.aspectRatio,
        resolution: params.resolution ?? null,
        style_id: params.styleId ?? null,
        style_prompt: params.stylePrompt ?? null,
        source_images: params.sourceImages ?? null,
        model: params.model ?? null,
        steps: params.steps ?? null,
    });
}

/**
 * List generated images for the gallery
 */
export async function imagineListImages(
    limit?: number,
    offset?: number,
    favoritesOnly?: boolean
): Promise<GeneratedImage[]> {
    return commandClient.directImagineListImages(limit ?? null, offset ?? null, favoritesOnly ?? null);
}

/**
 * Search generated images by prompt
 */
export async function imagineSearchImages(query: string): Promise<GeneratedImage[]> {
    return commandClient.directImagineSearchImages(query);
}

/**
 * Toggle favorite status for an image
 */
export async function imagineToggleFavorite(imageId: string): Promise<boolean> {
    return commandClient.directImagineToggleFavorite(imageId);
}

/**
 * Delete a generated image
 */
export async function imagineDeleteImage(imageId: string): Promise<void> {
    await commandClient.directImagineDeleteImage(imageId);
}

/**
 * Get gallery statistics
 */
export async function imagineGetStats(): Promise<ImagineStats> {
    return await commandClient.directImagineGetStats() as unknown as ImagineStats;
}
