#[cfg(feature = "mlx")]
#[tokio::main]
async fn main() {
    let mut args = std::env::args_os().skip(1);
    let app_data_dir = args
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: provision_mlx_runtime <app-data-dir> <uv-path>");
    let uv_path = args
        .next()
        .map(std::path::PathBuf::from)
        .expect("usage: provision_mlx_runtime <app-data-dir> <uv-path>");
    assert!(args.next().is_none(), "unexpected extra arguments");

    let engine = tauri_app_lib::engine::engine_mlx::MlxEngine::new();
    engine.set_app_data_dir(app_data_dir);
    engine.set_uv_path(uv_path);
    engine.bootstrap().await.expect("MLX provisioning failed");
    assert!(engine.is_bootstrapped(), "MLX environment is not ready");
    println!("MLX runtime provisioned and validated.");
}

#[cfg(not(feature = "mlx"))]
fn main() {
    panic!("provision_mlx_runtime requires --features mlx");
}
