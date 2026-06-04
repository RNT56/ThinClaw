import { writeFile, BaseDirectory, exists } from '@tauri-apps/plugin-fs';

/**
 * Downloads an image from an asset or remote URL and writes it directly to the Downloads folder.
 * @param url The asset:// or http:// URL of the image
 * @param filename The desired filename (e.g., "my-image.png")
 * @returns The final filename that was saved (e.g., "my-image (1).png")
 */
export async function downloadImageToDisk(url: string, filename: string): Promise<string> {
    try {
        // 1. Fetch the data from the local asset server or remote
        const response = await fetch(url);
        if (!response.ok) throw new Error(`Failed to fetch image: ${response.statusText}`);

        const buffer = await response.arrayBuffer();

        // 2. Convert to Uint8Array (required for the Tauri IPC bridge)
        const contents = new Uint8Array(buffer);

        // 3. Clean the filename (remove illegal OS characters)
        const sanitizedFilename = filename.replace(/[/\\?%*:|"<>]/g, '-');

        // 4. Get a unique filename to avoid overwrites
        const uniqueFilename = await getUniqueFilename(sanitizedFilename);

        // 5. Write directly to the user's Downloads directory
        await writeFile(uniqueFilename, contents, {
            baseDir: BaseDirectory.Download
        });

        return uniqueFilename;
    } catch (error) {
        console.error('Tauri FS Write Error:', error);
        throw error;
    }
}

/**
 * Advanced: Generates a unique filename if the file already exists.
 * e.g., "image.png" -> "image (1).png"
 */
export async function getUniqueFilename(filename: string): Promise<string> {
    try {
        let candidate = filename;
        let counter = 1;
        const extIndex = filename.lastIndexOf('.');
        const name = extIndex !== -1 ? filename.substring(0, extIndex) : filename;
        const ext = extIndex !== -1 ? filename.substring(extIndex) : '';

        // Check if file exists, if so, increment counter
        while (await exists(candidate, { baseDir: BaseDirectory.Download })) {
            candidate = `${name} (${counter})${ext}`;
            counter++;
        }
        return candidate;
    } catch (e) {
        // Fallback if exists() fails (e.g. permission error), just return original
        console.warn("Failed to check file existence, using original name:", e);
        return filename;
    }
}
