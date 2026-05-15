fn main() {
    let output_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/src/lib/bindings.ts");
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
}
