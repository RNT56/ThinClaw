//! Build script for ThinClaw Desktop.
//!
//! This script runs at compile time and handles:
//! 1. Tauri build setup
//! 2. Printing feature-flag information for CI logging
//! 3. Emitting conditional compile instructions based on the active engine

use serde_json::Value;
use std::path::PathBuf;

fn manifest_string<'a>(manifest: &'a Value, path: &[&str]) -> &'a str {
    let mut value = manifest;
    for component in path {
        value = value
            .get(component)
            .unwrap_or_else(|| panic!("engine-manifest.json is missing {}", path.join(".")));
    }
    value
        .as_str()
        .unwrap_or_else(|| panic!("engine-manifest.json {} must be a string", path.join(".")))
}

fn main() {
    let manifest_path = PathBuf::from("../engine-manifest.json");
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", manifest_path.display())),
    )
    .unwrap_or_else(|error| panic!("invalid {}: {error}", manifest_path.display()));

    for (name, path) in [
        ("THINCLAW_UV_VERSION", &["uv", "version"][..]),
        ("THINCLAW_PYTHON_VERSION", &["python", "version"][..]),
        (
            "THINCLAW_LLAMA_CPP_VERSION",
            &["engines", "llamacpp", "version"][..],
        ),
        (
            "THINCLAW_MLX_SERVER_VERSION",
            &["engines", "mlx", "version"][..],
        ),
        (
            "THINCLAW_MLX_MINIMUM_MACOS",
            &["engines", "mlx", "minimumMacosVersion"][..],
        ),
        ("THINCLAW_VLLM_VERSION", &["engines", "vllm", "version"][..]),
        (
            "THINCLAW_VLLM_TORCH_BACKEND",
            &["engines", "vllm", "torchBackend"][..],
        ),
        (
            "THINCLAW_VLLM_MINIMUM_GLIBC",
            &["engines", "vllm", "minimumGlibcVersion"][..],
        ),
        (
            "THINCLAW_VLLM_MINIMUM_COMPUTE_CAPABILITY",
            &["engines", "vllm", "minimumComputeCapability"][..],
        ),
    ] {
        println!(
            "cargo:rustc-env={name}={}",
            manifest_string(&manifest, path)
        );
    }
    let python_version = manifest_string(&manifest, &["python", "version"]);
    let python_abi = python_version
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".");
    assert!(
        python_abi.split('.').count() == 2,
        "engine-manifest.json python.version must contain major and minor components"
    );
    println!("cargo:rustc-env=THINCLAW_PYTHON_ABI={python_abi}");
    let mlx_packages = manifest
        .pointer("/engines/mlx/resolvedPackages")
        .and_then(Value::as_object)
        .unwrap_or_else(|| {
            panic!("engine-manifest.json engines.mlx.resolvedPackages must be an object")
        });
    println!(
        "cargo:rustc-env=THINCLAW_MLX_RUNTIME_PACKAGES={}",
        serde_json::to_string(mlx_packages).expect("MLX package manifest should serialize")
    );

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
    println!("cargo:rerun-if-changed=../engine-manifest.json");
    println!("cargo:rerun-if-changed=../runtime/mlx/requirements.lock");
    println!("cargo:rerun-if-changed=../runtime/vllm/requirements.lock");
}
