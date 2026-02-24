//! Build script for Scrappy.
//!
//! This script runs at compile time and handles:
//! 1. Tauri build setup
//! 2. Printing feature-flag information for CI logging
//! 3. Emitting conditional compile instructions based on the active engine

fn main() {
    // Standard Tauri build
    tauri_build::build();

    // -----------------------------------------------------------------------
    // Feature-flag diagnostics — printed during CI builds for visibility
    // -----------------------------------------------------------------------

    let engine = if cfg!(feature = "mlx") {
        "mlx"
    } else if cfg!(feature = "vllm") {
        "vllm"
    } else if cfg!(feature = "llamacpp") {
        "llamacpp"
    } else if cfg!(feature = "ollama") {
        "ollama"
    } else {
        "none"
    };

    println!("cargo:warning=Active inference engine: {}", engine);

    // Re-run if any feature flag changes
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_LLAMACPP");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_MLX");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_VLLM");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_OLLAMA");
}
