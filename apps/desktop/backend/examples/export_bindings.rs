fn main() {
    let backend_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output_path = backend_path.join("../frontend/src/lib/bindings.ts");
    let builder = tauri_app_lib::setup::commands::specta_builder();
    builder
        .export(
            specta_typescript::Typescript::default()
                .bigint(specta_typescript::BigIntExportBehavior::Number),
            &output_path,
        )
        .expect("failed to export frontend bindings");
    tauri_app_lib::sanitize_typescript_bindings(output_path.to_str().expect("utf8 path"))
        .expect("failed to sanitize frontend bindings");

    let route_matrix_path = backend_path.join("../documentation/remote-gateway-route-matrix.md");
    tauri_app_lib::thinclaw::bridge::write_route_matrix_document(route_matrix_path)
        .expect("failed to export remote gateway route matrix");
}
