use super::*;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn injected_and_configured_databases_initialize() {
    use crate::db::Database as _;
    use crate::db::SettingsStore as _;
    use std::time::Duration;

    async fn bounded<T>(label: &'static str, future: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(30), future)
            .await
            .unwrap_or_else(|_| panic!("{label} timed out"))
    }

    let settings = crate::settings::Settings {
        llm_backend: Some("openai_compatible".to_string()),
        openai_compatible_base_url: Some("http://localhost:12345/v1".to_string()),
        ..crate::settings::Settings::default()
    };
    let temp = tempfile::TempDir::new().expect("temp dir");
    let backend = bounded(
        "opening injected database",
        crate::db::libsql::LibSqlBackend::new_local(&temp.path().join("shared.db")),
    )
    .await
    .expect("open shared database");
    bounded("migrating injected database", backend.run_migrations())
        .await
        .expect("run migrations");
    let database: Arc<dyn Database> = Arc::new(backend);
    let mut persisted_settings = settings.clone();
    persisted_settings.secrets.master_key_source = crate::settings::SecretsMasterKeySource::None;
    bounded(
        "seeding injected database settings",
        database.set_all_settings("default", &persisted_settings.to_db_map()),
    )
    .await
    .expect("seed injected database settings");
    let mut builder = AppBuilder::new(
        bounded(
            "building injected database config",
            Config::from_test_settings(&settings),
        )
        .await
        .expect("build test config"),
        AppBuilderFlags::default(),
        None,
        Arc::new(LogBroadcaster::new()),
    )
    .with_database(Arc::clone(&database));

    bounded("initializing injected database", builder.init_database())
        .await
        .expect("initialize injected database");

    assert!(Arc::ptr_eq(
        builder.db().expect("database retained"),
        &database
    ));

    let mut config = bounded(
        "building configured database config",
        Config::from_test_settings(&settings),
    )
    .await
    .expect("build test config");
    config.database.backend = crate::config::DatabaseBackend::LibSql;
    let configured_path = temp.path().join("configured.db");
    let configured_seed = bounded(
        "opening configured database seed",
        crate::db::libsql::LibSqlBackend::new_local(&configured_path),
    )
    .await
    .expect("open configured database seed");
    bounded(
        "migrating configured database seed",
        configured_seed.run_migrations(),
    )
    .await
    .expect("migrate configured database seed");
    bounded(
        "seeding configured database settings",
        configured_seed.set_all_settings("default", &persisted_settings.to_db_map()),
    )
    .await
    .expect("seed configured database settings");
    drop(configured_seed);
    config.database.libsql_path = Some(configured_path);
    let mut configured_builder = AppBuilder::new(
        config,
        AppBuilderFlags::default(),
        None,
        Arc::new(LogBroadcaster::new()),
    );

    bounded(
        "initializing configured database",
        configured_builder.init_database(),
    )
    .await
    .expect("initialize configured database");
    assert!(configured_builder.db().is_some());
}

#[test]
fn restricted_modes_disable_background_processes() {
    assert_eq!(
        process_registration_mode("sandboxed"),
        RuntimeExecRegistrationMode::Disabled
    );
    assert_eq!(
        process_registration_mode("project"),
        RuntimeExecRegistrationMode::Disabled
    );
    assert_eq!(
        process_registration_mode("unrestricted"),
        RuntimeExecRegistrationMode::LocalHost
    );
}

#[test]
fn execute_code_requires_real_isolation_in_restricted_modes() {
    assert_eq!(
        execute_code_registration_mode("sandboxed", true),
        RuntimeExecRegistrationMode::DockerSandbox
    );
    assert_eq!(
        execute_code_registration_mode("sandboxed", false),
        RuntimeExecRegistrationMode::Disabled
    );
    assert_eq!(
        execute_code_registration_mode("project", true),
        RuntimeExecRegistrationMode::Disabled
    );
    assert_eq!(
        execute_code_registration_mode("unrestricted", false),
        RuntimeExecRegistrationMode::LocalHost
    );
}

#[test]
fn pi_os_lite_runtime_blocks_desktop_autonomy_registration() {
    assert_eq!(
        desktop_autonomy_headless_blocker_for("pi-os-lite-64", false),
        Some("pi-os-lite-64")
    );
    assert_eq!(
        desktop_autonomy_headless_blocker_for("raspberry-pi-os-lite", false),
        Some("pi-os-lite-64")
    );
    assert_eq!(
        desktop_autonomy_headless_blocker_for("remote", true),
        Some("headless")
    );
    assert_eq!(desktop_autonomy_headless_blocker_for("remote", false), None);
}
