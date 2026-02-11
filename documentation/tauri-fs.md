# Tauri v2 File System (FS) Implementation Guide

This document outlines the full specification and implementation strategy for integrating `tauri-plugin-fs` into the Scrappy application. This approach bypasses browser-level download limitations and provides direct, silent file-saving capabilities.

## 1. Prerequisites & Installation

### Backend (Rust)
Ensure the plugin is registered in `src-tauri/src/lib.rs`:

```rust
// src-tauri/src/lib.rs
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init()) // Ensure this is present
        // ... other plugins
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Add the dependency to `src-tauri/Cargo.toml`:
```toml
[dependencies]
tauri-plugin-fs = "2"
```

### Frontend (TypeScript)
Install the JavaScript bindings:
```bash
npm install @tauri-apps/plugin-fs
```

---

## 2. Security & Capability Configuration

Tauri v2 uses a strict **Access Control List (ACL)**. To allow writing to the Downloads folder, you must explicitly define the scopes in your capability files (e.g., `src-tauri/capabilities/default.json`).

### Recommended `default.json` Entry
Tauri v2 requires command-level permissions AND path-level scopes. You can define scopes globally for all FS operations or per-command for tighter security.

**Configuration Strategy:**
1.  **Grant Command Access**: Add `fs:allow-write-file` and `fs:allow-read-file`.
2.  **Define Scopes**: Global scopes match all FS commands. Use `$DOWNLOAD/**` for download folder access.

```json
{
  "identifier": "default",
  "permissions": [
    "fs:default",
    {
      "identifier": "fs:allow-write-file",
      "allow": [
        { "path": "$DOWNLOAD/**" }
      ]
    },
    {
      "identifier": "fs:allow-read-file",
      "allow": [
        { "path": "$DOWNLOAD/**" }
      ]
    },
    {
      "identifier": "fs:scope",
      "allow": [
        { "path": "$DOWNLOAD/**" }
      ]
    }
  ]
}
```
*Note: Deny rules (if added) always supersede allow rules in Tauri v2.*

---

## 3. Implementation Pattern

The following utility function provides a robust wrapper for saving binary data (like images) directly to the local filesystem.

### Implementation Logic
```typescript
import { writeFile, BaseDirectory } from '@tauri-apps/plugin-fs';

/**
 * Downloads an image from an asset or remote URL and writes it directly to the Downloads folder.
 * @param url The asset:// or http:// URL of the image
 * @param filename The desired filename (e.g., "my-image.png")
 */
export async function downloadImageToDisk(url: string, filename: string): Promise<void> {
    try {
        // 1. Fetch the data from the local asset server or remote
        const response = await fetch(url);
        if (!response.ok) throw new Error(`Failed to fetch image: ${response.statusText}`);
        
        const buffer = await response.arrayBuffer();
        
        // 2. Convert to Uint8Array (required for the Tauri IPC bridge)
        const contents = new Uint8Array(buffer);

        // 3. Clean the filename (remove illegal OS characters)
        const sanitizedFilename = filename.replace(/[/\\?%*:|"<>]/g, '-');

        // 4. Write directly to the user's Downloads directory
        await writeFile(sanitizedFilename, contents, { 
            baseDir: BaseDirectory.Download 
        });

        console.log(`Successfully saved ${sanitizedFilename} to Downloads`);
    } catch (error) {
        console.error('Tauri FS Write Error:', error);
        throw error;
    }
        console.log(`Successfully saved ${sanitizedFilename} to Downloads`);
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
    const { exists } = await import('@tauri-apps/plugin-fs');
    
    let candidate = filename;
    let counter = 1;
    const extIndex = filename.lastIndexOf('.');
    const name = extIndex !== -1 ? filename.substring(0, extIndex) : filename;
    const ext = extIndex !== -1 ? filename.substring(extIndex) : '';

    while (await exists(candidate, { baseDir: BaseDirectory.Download })) {
        candidate = `${name} (${counter})${ext}`;
        counter++;
    }
    return candidate;
}
```

---

## 4. Cross-Platform Path Resolution (Auto-Discovery)

One of the most powerful features of Tauri is its ability to **auto-discover** correct system paths across different machines and operating systems. You should **never** hardcode an absolute path (e.g., `/Users/mt/Downloads`).

### How Tauri Resolves Paths
*   **The `$DOWNLOAD` Variable**: In the configuration (`capabilities.json`), this variable is a placeholder that Tauri resolves at runtime by querying the OS.
*   **The `BaseDirectory` Enum**: In TypeScript, using `BaseDirectory.Download` ensures that the file is written to the correct location regardless of the username or OS (Windows, macOS, or Linux).

| Developer machine | User machine (Windows) | User machine (Linux) |
| :--- | :--- | :--- |
| `/Users/mt/Downloads` | `C:\Users\John\Downloads` | `/home/alice/Downloads` |
| **Tauri sees:** `$DOWNLOAD` | **Tauri sees:** `$DOWNLOAD` | **Tauri sees:** `$DOWNLOAD` |

---

## 5. Why Use This Over Blob URLs?

| Feature | Blob URL Method | Tauri FS Method |
| :--- | :--- | :--- |
| **Silent Save** | No (Often triggers Save As dialog) | **Yes** (Saves directly to disk) |
| **Bulk Experience** | Poor (Multiple popups/dialogs) | **Excellent** (Processes 100% in background) |
| **Path Control** | Browser decides location | **App decides location** |
| **Memory** | High (Stores full blobs in RAM) | Low (Streams data directly to disk) |
| **Complexity** | Low | Medium (Requires ACL config) |

---

## 5. Troubleshooting Common Issues

1. **"Path Forbidden" Error**: 
   - Check `capabilities/default.json`. Ensure `$DOWNLOAD/**` is in the `allow` list for the specific operation (read/write).
   - Ensure the `tauri-plugin-fs` is initialized in `lib.rs`.

2. **"Incorrect Body Type"**: 
   - Ensure you are passing a `Uint8Array` to `writeFile`, not a `Blob`, `ArrayBuffer`, or `number[]`.

3. **File Permissions**: 
   - On macOS/Linux, ensure the app process has permission to write to the folder (usually granted automatically to the Downloads folder for user-space apps).

---

## 6. Current Implementation (Blob URL Method)

This is the **currently active** method used in Scrappy. It is documented here for reference and in case a revert is needed during testing of the `tauri:fs` upgrade.

### Pattern Logic
This method relies on the browser's ability to create a temporary memory link to file data.

```typescript
const handleDownload = async (image: GeneratedImage) => {
    try {
        // 1. Convert local path to a fetchable asset URL
        const assetUrl = convertFileSrc(image.filePath);
        
        // 2. Fetch the data into a Blob
        const response = await fetch(assetUrl);
        const blob = await response.blob();
        
        // 3. Create a temporary 'blob:' URL
        const url = URL.createObjectURL(blob);

        // 4. Trigger a simulated click on a hidden anchor tag
        const link = document.createElement('a');
        link.href = url;
        link.download = `image-name.png`;
        document.body.appendChild(link);
        link.click();
        
        // 5. Cleanup
        document.body.removeChild(link);
        URL.revokeObjectURL(url);
        
        toast.success("Saved to Downloads");
    } catch (err) {
        console.error('Download failed:', err);
    }
};
```

### Why We Use This Currently
*   **Protocol Safety**: `asset://` URLs cannot be downloaded directly via `href` because the WebView tries to navigate to them (causing a blank screen). Converting them to `blob:` URLs tells the WebView this is data to be saved, not a page to visit.
*   **No Permissions Needed**: This method doesn't require any special `capabilities.json` configuration, making it the most stable "zero-config" solution for single-image downloads.
