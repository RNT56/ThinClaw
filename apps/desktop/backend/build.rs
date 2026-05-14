//! Build script for ThinClaw Desktop.
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

    // -----------------------------------------------------------------------
    // Explicit watch scope — CRITICAL for dev stability
    // -----------------------------------------------------------------------
    // Without explicit rerun-if-changed directives, cargo's default behaviour
    // is to re-run this build script (and therefore trigger a full Tauri dev
    // restart) whenever ANY file in the package directory changes.
    //
    // In practice this means: if the agent writes a file (e.g. bitcoin_article.md)
    // into the backend/ directory while running in unrestricted workspace mode,
    // cargo detects the change, rebuilds, and Tauri kills the running app.
    //
    // By listing only Rust source and manifest files here we restrict the
    // watcher to changes that actually require a recompile.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../../../Cargo.toml");
    println!("cargo:rerun-if-changed=../../../src");
    println!("cargo:rerun-if-changed=../../../crates");
    println!("cargo:rerun-if-changed=tauri.conf.json");
}
