import { invoke } from '@tauri-apps/api/core';

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
    return invoke<GeneratedImage>('direct_imagine_generate', {
        params: {
            prompt: params.prompt,
            provider: params.provider,
            aspect_ratio: params.aspectRatio,
            resolution: params.resolution,
            style_id: params.styleId,
            style_prompt: params.stylePrompt,
            source_images: params.sourceImages,
            model: params.model,
            steps: params.steps,
        }
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
    return invoke<GeneratedImage[]>('direct_imagine_list_images', {
        limit,
        offset,
        favoritesOnly
    });
}

/**
 * Search generated images by prompt
 */
export async function imagineSearchImages(query: string): Promise<GeneratedImage[]> {
    return invoke<GeneratedImage[]>('direct_imagine_search_images', { query });
}

/**
 * Toggle favorite status for an image
 */
export async function imagineToggleFavorite(imageId: string): Promise<boolean> {
    return invoke<boolean>('direct_imagine_toggle_favorite', { imageId });
}

/**
 * Delete a generated image
 */
export async function imagineDeleteImage(imageId: string): Promise<void> {
    return invoke<void>('direct_imagine_delete_image', { imageId });
}

/**
 * Get gallery statistics
 */
export async function imagineGetStats(): Promise<ImagineStats> {
    return invoke<ImagineStats>('direct_imagine_get_stats');
}

/**
 * Convert a file path to a Tauri asset URL
 */
export function getAssetUrl(filePath: string): string {
    // Use Tauri's asset protocol to serve local files
    return `asset://localhost/${encodeURIComponent(filePath)}`;
}
