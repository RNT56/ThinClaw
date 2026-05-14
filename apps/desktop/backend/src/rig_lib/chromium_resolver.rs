use std::path::PathBuf;
use tauri::Manager;

#[cfg(target_os = "macos")]
fn get_platform_exec_path() -> &'static str {
    "chrome-mac/Chromium.app/Contents/MacOS/Chromium"
}

#[cfg(not(target_os = "macos"))]
fn get_platform_exec_path() -> &'static str {
    // Fallback/TODO for other OS if needed, though script only supported mac for now
    "chrome-linux/chrome"
}

pub async fn ensure_chromium(app: Option<&tauri::AppHandle>) -> Result<PathBuf, String> {
    let exec_path = get_platform_exec_path();

    // 1. Try to resolve via AppHandle (Prod/Dev runtime)
    if let Some(app) = app {
        let resource_dir = app
            .path()
            .resource_dir()
            .map_err(|e| format!("Failed to get resource dir: {}", e))?;

        // Structure: <resource_dir>/resources/chromium/<exec_path>
        // Note: verify if tauri flattens "resources/chromium/**" or keeps it.
        // Usually, if you include "resources/chromium/**", it copies that folder structure.
        // So valid path: resource_dir + "resources/chromium" + exec_path

        let path = resource_dir
            .join("resources")
            .join("chromium")
            .join(exec_path);
        if path.exists() {
            println!("Found bundled Chromium at: {:?}", path);
            return Ok(path);
        } else {
            println!("Bundled Chromium NOT found at: {:?}", path);
            // Fallback to check if it flattened?
        }
    }

    // 2. Fallback for Tests / Dev (cwd is usually backend/ or project root)
    // In tests, CWD is backend/.
    // In dev run, CWD might be project root.

    // Check relative to backend/resources
    let dev_paths = vec!["backend/resources/chromium", "resources/chromium"];

    for base in dev_paths {
        let path = std::path::Path::new(base).join(exec_path);
        if let Ok(abs_path) = std::fs::canonicalize(&path) {
            if abs_path.exists() {
                println!("Found local Dev Chromium at: {:?}", abs_path);
                return Ok(abs_path);
            }
        }
    }

    Err(
        "Could not find Chromium binary. Please run 'backend/scripts/setup_chromium.sh'."
            .to_string(),
    )
}
