//! Exports the gateway's v1 mobile-contract OpenAPI document.
//!
//! Mirrors the runtime-contracts snapshot pattern
//! (`crates/thinclaw-runtime-contracts/examples/export_contracts.rs`):
//!
//! ```bash
//! cargo run --bin export-openapi -- generate   # rewrite the committed snapshot
//! cargo run --bin export-openapi -- check      # fail if the snapshot drifted
//! ```
//!
//! The snapshot at `clients/openapi/thinclaw-gateway.openapi.json` is the
//! input for the Swift client generator (`apps/ios/scripts/generate-api.sh`).
//! See `docs/MOBILE_APP.md`.

use std::path::PathBuf;
use std::process::ExitCode;

use thinclaw::channels::web::openapi::gateway_openapi;

const SNAPSHOT_RELATIVE: &str = "clients/openapi/thinclaw-gateway.openapi.json";

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SNAPSHOT_RELATIVE)
}

fn render() -> String {
    let doc = gateway_openapi();
    let mut json = serde_json::to_string_pretty(&doc).expect("OpenAPI document serializes");
    json.push('\n');
    json
}

fn main() -> ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let path = snapshot_path();
    let rendered = render();

    match mode.as_str() {
        "generate" => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create clients/openapi");
            }
            std::fs::write(&path, rendered).expect("write OpenAPI snapshot");
            println!("wrote {}", path.display());
            ExitCode::SUCCESS
        }
        "check" => {
            let committed = match std::fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(err) => {
                    eprintln!(
                        "missing committed OpenAPI snapshot at {} ({err}); run \
                         `cargo run --bin export-openapi -- generate`",
                        path.display()
                    );
                    return ExitCode::FAILURE;
                }
            };
            if committed == rendered {
                println!("OpenAPI snapshot is up to date");
                ExitCode::SUCCESS
            } else {
                eprintln!(
                    "OpenAPI snapshot at {} is stale; run \
                     `cargo run --bin export-openapi -- generate` and commit the result",
                    path.display()
                );
                ExitCode::FAILURE
            }
        }
        other => {
            eprintln!("usage: export-openapi <generate|check> (got {other:?})");
            ExitCode::FAILURE
        }
    }
}
