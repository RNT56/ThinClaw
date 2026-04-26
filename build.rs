// Build scripts are compile-time only — panics abort the build with clear errors.
#![allow(clippy::unwrap_used)]

//! Build script: compile WASM extensions and embed registry catalog.
//!
//! Do not commit compiled WASM binaries — they are a supply chain risk.
//!
//! ## Default behavior
//! Builds telegram.wasm from channels-src/telegram and embeds the registry
//! catalog JSON into the binary via `include_str!`.
//!
//! ## `bundled-wasm` feature
//! When compiled with `--features bundled-wasm`, **all** WASM extensions
//! (10 tools + 4 channels) are built and embedded into the binary via
//! `include_bytes!`. At install time, they are extracted to
//! `~/.thinclaw/{tools,channels}/` — zero network dependency.
//!
//! Reproducible build:
//!   cargo build --release
//!   cargo build --release --features bundled-wasm   # air-gapped
//!
//! Prerequisites: rustup target add wasm32-wasip2, cargo install wasm-tools

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(&manifest_dir);

    // ── Embed registry manifests ────────────────────────────────────────
    embed_registry_catalog(&root);

    // ── Feature: bundled-wasm ───────────────────────────────────────────
    // Build ALL WASM extensions and generate include_bytes! entries.
    if env::var("CARGO_FEATURE_BUNDLED_WASM").is_ok() {
        build_all_wasm_extensions(&root);
    } else {
        // Default: only build Telegram channel WASM
        build_telegram_channel(&root);
    }
}

/// Build only the Telegram channel WASM (default build behavior).
fn build_telegram_channel(root: &Path) {
    let channel_dir = root.join("channels-src/telegram");
    let wasm_out = channel_dir.join("telegram.wasm");

    // Rerun when channel source or build script changes
    println!("cargo:rerun-if-changed=channels-src/telegram/src");
    println!("cargo:rerun-if-changed=channels-src/telegram/Cargo.toml");
    println!("cargo:rerun-if-changed=wit/channel.wit");

    if !channel_dir.is_dir() {
        return;
    }

    // Build WASM module
    let status = match Command::new("cargo")
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
            channel_dir.join("Cargo.toml").to_str().unwrap(),
        ])
        .current_dir(root)
        .status()
    {
        Ok(s) => s,
        Err(_) => {
            eprintln!(
                "cargo:warning=Telegram channel build failed. Run: ./channels-src/telegram/build.sh"
            );
            return;
        }
    };

    if !status.success() {
        eprintln!(
            "cargo:warning=Telegram channel build failed. Run: ./channels-src/telegram/build.sh"
        );
        return;
    }

    let raw_wasm = channel_dir.join("target/wasm32-wasip2/release/telegram_channel.wasm");
    if !raw_wasm.exists() {
        eprintln!(
            "cargo:warning=Telegram WASM output not found at {:?}",
            raw_wasm
        );
        return;
    }

    // Convert to component and strip (wasm-tools)
    let component_ok = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            raw_wasm.to_str().unwrap(),
            "-o",
            wasm_out.to_str().unwrap(),
        ])
        .current_dir(root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !component_ok {
        // Fallback: copy raw module if wasm-tools unavailable
        if std::fs::copy(&raw_wasm, &wasm_out).is_err() {
            eprintln!("cargo:warning=wasm-tools not found. Run: cargo install wasm-tools");
        }
    } else {
        // Strip debug info (use temp file to avoid clobbering)
        let stripped = wasm_out.with_extension("wasm.stripped");
        let strip_ok = Command::new("wasm-tools")
            .args([
                "strip",
                wasm_out.to_str().unwrap(),
                "-o",
                stripped.to_str().unwrap(),
            ])
            .current_dir(root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if strip_ok {
            let _ = std::fs::rename(&stripped, &wasm_out);
        }
    }
}

/// Build all WASM extensions and generate `bundled_wasm_entries.rs` for `include_bytes!`.
///
/// Reads every manifest from `registry/{tools,channels}/*.json`, builds the
/// corresponding WASM component, and copies it (plus capabilities JSON) to
/// `$OUT_DIR/wasm_bundles/`. Then generates a Rust source file that pairs each
/// extension name with `include_bytes!` references.
fn build_all_wasm_extensions(root: &Path) {
    use std::fs;

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let bundles_dir = out_dir.join("wasm_bundles");
    fs::create_dir_all(&bundles_dir).unwrap();

    // Rerun if any extension source changes
    println!("cargo:rerun-if-changed=tools-src");
    println!("cargo:rerun-if-changed=channels-src");

    // Collect all manifests
    let registry_dir = root.join("registry");
    let mut entries: Vec<BundleEntry> = Vec::new();

    for (subdir, kind) in &[("tools", "tool"), ("channels", "channel")] {
        let dir = registry_dir.join(subdir);
        if !dir.is_dir() {
            continue;
        }

        let mut manifest_files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("json")
            })
            .collect();
        manifest_files.sort_by_key(|e| e.file_name());

        for entry in manifest_files {
            let content = match fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let manifest: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "cargo:warning=Failed to parse {}: {}",
                        entry.path().display(),
                        e
                    );
                    continue;
                }
            };

            let name = manifest["name"].as_str().unwrap_or("").to_string();
            let source_dir = manifest["source"]["dir"].as_str().unwrap_or("").to_string();
            let caps_file = manifest["source"]["capabilities"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let crate_name = manifest["source"]["crate_name"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if name.is_empty() || source_dir.is_empty() || crate_name.is_empty() {
                continue;
            }

            let abs_source_dir = root.join(&source_dir);
            if !abs_source_dir.is_dir() {
                eprintln!(
                    "cargo:warning=bundled-wasm: Source dir '{}' not found for '{}', skipping",
                    source_dir, name
                );
                continue;
            }

            // Build the WASM component
            eprintln!(
                "cargo:warning=bundled-wasm: Building {} ({})...",
                name, kind
            );

            let build_ok = Command::new("cargo")
                .args([
                    "build",
                    "--release",
                    "--target",
                    "wasm32-wasip2",
                    "--manifest-path",
                    abs_source_dir.join("Cargo.toml").to_str().unwrap(),
                ])
                .current_dir(root)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if !build_ok {
                eprintln!(
                    "cargo:warning=bundled-wasm: Build failed for '{}', skipping",
                    name
                );
                continue;
            }

            // Find the built artifact
            let snake_crate = crate_name.replace('-', "_");
            let mut wasm_src = None;
            for triple in &["wasm32-wasip2", "wasm32-wasip1", "wasm32-wasi"] {
                let candidate = abs_source_dir
                    .join("target")
                    .join(triple)
                    .join("release")
                    .join(format!("{}.wasm", snake_crate));
                if candidate.exists() {
                    wasm_src = Some(candidate);
                    break;
                }
            }

            let wasm_src = match wasm_src {
                Some(p) => p,
                None => {
                    eprintln!(
                        "cargo:warning=bundled-wasm: No WASM output found for '{}', skipping",
                        name
                    );
                    continue;
                }
            };

            // Copy WASM to bundles dir
            let wasm_dst = bundles_dir.join(format!("{}.wasm", name));
            if let Err(e) = fs::copy(&wasm_src, &wasm_dst) {
                eprintln!(
                    "cargo:warning=bundled-wasm: Failed to copy WASM for '{}': {}",
                    name, e
                );
                continue;
            }

            // Copy capabilities if present
            let caps_src = abs_source_dir.join(&caps_file);
            let has_caps = if caps_src.exists() {
                let caps_dst = bundles_dir.join(format!("{}.capabilities.json", name));
                fs::copy(&caps_src, &caps_dst).is_ok()
            } else {
                false
            };

            entries.push(BundleEntry {
                name,
                kind: kind.to_string(),
                has_caps,
            });

            eprintln!(
                "cargo:warning=bundled-wasm: ✓ {}",
                entries.last().unwrap().name
            );
        }
    }

    // Generate Rust source: bundled_wasm_entries.rs
    let mut code = String::new();
    code.push_str("// Auto-generated by build.rs — do not edit.\n");
    code.push_str("// Contains include_bytes! entries for all bundled WASM extensions.\n\n");
    code.push_str("/// Bundled WASM extension entry: (name, kind, wasm_bytes, caps_bytes)\n");
    code.push_str("pub const BUNDLED_ENTRIES: &[BundledEntry] = &[\n");

    for entry in &entries {
        let wasm_path = bundles_dir.join(format!("{}.wasm", entry.name));
        let caps_expr = if entry.has_caps {
            let caps_path = bundles_dir.join(format!("{}.capabilities.json", entry.name));
            format!("Some(include_bytes!({:?}))", caps_path.to_str().unwrap())
        } else {
            "None".to_string()
        };

        code.push_str(&format!(
            "    ({name:?}, {kind:?}, include_bytes!({wasm:?}), {caps}),\n",
            name = entry.name,
            kind = entry.kind,
            wasm = wasm_path.to_str().unwrap(),
            caps = caps_expr,
        ));
    }

    code.push_str("];\n");

    let entries_path = out_dir.join("bundled_wasm_entries.rs");
    fs::write(&entries_path, code).unwrap();

    eprintln!(
        "cargo:warning=bundled-wasm: Embedded {} extensions into binary",
        entries.len()
    );
}

struct BundleEntry {
    name: String,
    kind: String,
    has_caps: bool,
}

/// Collect all registry manifests into a single JSON blob at compile time.
///
/// Output: `$OUT_DIR/embedded_catalog.json` with structure:
/// ```json
/// { "tools": [...], "channels": [...], "bundles": {...} }
/// ```
fn embed_registry_catalog(root: &Path) {
    use std::fs;

    let registry_dir = root.join("registry");

    // Rerun if the bundles file changes (per-file watches for tools/channels
    // are emitted inside collect_json_files to track content changes reliably).
    println!("cargo:rerun-if-changed=registry/_bundles.json");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_path = out_dir.join("embedded_catalog.json");

    if !registry_dir.is_dir() {
        // No registry dir: write empty catalog
        fs::write(
            &out_path,
            r#"{"tools":[],"channels":[],"bundles":{"bundles":{}}}"#,
        )
        .unwrap();
        return;
    }

    let mut tools = Vec::new();
    let mut channels = Vec::new();

    // Collect tool manifests
    let tools_dir = registry_dir.join("tools");
    if tools_dir.is_dir() {
        collect_json_files(&tools_dir, &mut tools);
    }

    // Collect channel manifests
    let channels_dir = registry_dir.join("channels");
    if channels_dir.is_dir() {
        collect_json_files(&channels_dir, &mut channels);
    }

    // Read bundles
    let bundles_path = registry_dir.join("_bundles.json");
    let bundles_raw = if bundles_path.is_file() {
        fs::read_to_string(&bundles_path).unwrap_or_else(|_| r#"{"bundles":{}}"#.to_string())
    } else {
        r#"{"bundles":{}}"#.to_string()
    };

    // Build the combined JSON
    let catalog = format!(
        r#"{{"tools":[{}],"channels":[{}],"bundles":{}}}"#,
        tools.join(","),
        channels.join(","),
        bundles_raw,
    );

    fs::write(&out_path, catalog).unwrap();

    // Also embed providers.json for the provider catalog fallback
    let providers_src = registry_dir.join("providers.json");
    let providers_out = out_dir.join("providers_catalog.json");
    println!("cargo:rerun-if-changed=registry/providers.json");
    if providers_src.is_file() {
        fs::copy(&providers_src, &providers_out).expect("failed to copy providers.json to OUT_DIR");
    } else {
        fs::write(&providers_out, "[]").unwrap();
    }

    // Embed models.json for the model compat catalog fallback
    let models_src = registry_dir.join("models.json");
    let models_out = out_dir.join("models_catalog.json");
    println!("cargo:rerun-if-changed=registry/models.json");
    if models_src.is_file() {
        fs::copy(&models_src, &models_out).expect("failed to copy models.json to OUT_DIR");
    } else {
        fs::write(&models_out, r#"{"version":1,"models":[]}"#).unwrap();
    }
}

/// Read all .json files from a directory and push their raw contents into `out`.
fn collect_json_files(dir: &Path, out: &mut Vec<String>) {
    use std::fs;

    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file() && e.path().extension().and_then(|x| x.to_str()) == Some("json")
        })
        .collect();

    // Sort for deterministic output
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        // Emit per-file watch so Cargo reruns when file contents change
        println!("cargo:rerun-if-changed={}", entry.path().display());
        if let Ok(content) = fs::read_to_string(entry.path()) {
            out.push(content);
        }
    }
}
