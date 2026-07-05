//! Guards the committed OpenAPI snapshot against silent drift.
//!
//! Regenerate with `cargo run --bin export-openapi -- generate`.

use thinclaw::channels::web::openapi::gateway_openapi;

#[test]
fn committed_openapi_snapshot_matches_generated_document() {
    let committed_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/clients/openapi/thinclaw-gateway.openapi.json"
    );
    let committed = std::fs::read_to_string(committed_path).expect(
        "missing clients/openapi/thinclaw-gateway.openapi.json; run \
         `cargo run --bin export-openapi -- generate`",
    );

    let committed_value: serde_json::Value =
        serde_json::from_str(&committed).expect("committed snapshot is valid JSON");
    let generated_value =
        serde_json::to_value(gateway_openapi()).expect("generated document serializes");

    assert_eq!(
        committed_value, generated_value,
        "OpenAPI snapshot drifted from the code; run \
         `cargo run --bin export-openapi -- generate` and commit the result"
    );
}
