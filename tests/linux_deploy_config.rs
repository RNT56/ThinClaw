use std::path::Path;

fn repo_file(path: &str) -> String {
    let full_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&full_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", full_path.display()))
}

#[test]
fn deploy_files_do_not_reintroduce_legacy_gateway_port() {
    for path in [
        "Dockerfile",
        "deploy/docker-compose.yml",
        "deploy/docker-compose.postgres.yml",
        "deploy/setup.sh",
        "docs/DEPLOYMENT.md",
        "docs/EXTERNAL_DEPENDENCIES.md",
    ] {
        let contents = repo_file(path);
        assert!(
            !contents.contains("18789"),
            "{} must not reintroduce the legacy gateway port",
            path
        );
    }
}

#[test]
fn docker_compose_setup_and_docs_share_gateway_port() {
    let dockerfile = repo_file("Dockerfile");
    let compose = repo_file("deploy/docker-compose.yml");
    let setup = repo_file("deploy/setup.sh");
    let deployment_docs = repo_file("docs/DEPLOYMENT.md");

    assert!(dockerfile.contains("EXPOSE 3000"));
    assert!(dockerfile.contains("ARG THINCLAW_BINARY=thinclaw"));
    assert!(dockerfile.contains("ENTRYPOINT [\"/usr/local/bin/docker-entrypoint.sh\"]"));
    assert!(dockerfile.contains("CMD [\"run\", \"--skip-setup-check\"]"));
    assert!(compose.contains("ghcr.io/rnt56/thinclaw:latest"));
    assert!(compose.contains("GATEWAY_PORT: \"${GATEWAY_PORT:-3000}\""));
    assert!(compose.contains("localhost:$${GATEWAY_PORT:-3000}/api/health"));
    assert!(setup.contains("THINCLAW_PORT=\"${GATEWAY_PORT:-3000}\""));
    assert!(setup.contains("--mode <auto|native|docker>"));
    assert!(setup.contains("ExecStart=/usr/local/bin/thinclaw run --skip-setup-check"));
    assert!(setup.contains("THINCLAW_HOME=/var/lib/thinclaw/.thinclaw"));
    assert!(setup.contains("LIBSQL_PATH=/var/lib/thinclaw/.thinclaw/thinclaw.db"));
    assert!(setup.contains("http://localhost:$THINCLAW_PORT/api/health"));
    assert!(setup.contains("ROLLBACK_DIR"));
    assert!(setup.contains("backup_path"));
    assert!(setup.contains("rollback_setup"));
    assert!(setup.contains("wait_for_health"));
    assert!(setup.contains("THINCLAW_ROLLBACK_PACKAGES"));
    assert!(setup.contains("PACKAGES_INSTALLED_BY_SETUP"));
    assert!(setup.contains("rollback_installed_packages"));
    assert!(setup.contains("restore_service_state docker"));
    assert!(setup.contains("restore_ufw_state"));
    assert!(setup.contains("rollback_tailscale_state"));
    assert!(setup.contains("set_env_value .env GATEWAY_AUTH_TOKEN \"$TOKEN\""));
    assert!(setup.contains("read_env_value .env SECRETS_MASTER_KEY"));
    assert!(setup.contains("set_env_value .env SECRETS_MASTER_KEY \"$MASTER_KEY\""));
    assert!(setup.contains("set_env_value .env ONBOARD_COMPLETED true"));
    assert!(setup.contains("THINCLAW_FIREWALL_STRICT"));
    assert!(
        !setup.contains("ufw reset"),
        "installer must not reset existing firewall rules"
    );
    assert!(deployment_docs.contains("Code-backed default gateway port: `3000`"));
}

#[test]
fn public_compose_is_headless_persistent_and_bounded() {
    let dockerfile = repo_file("Dockerfile");
    let entrypoint = repo_file("deploy/docker-entrypoint.sh");
    let compose = repo_file("deploy/docker-compose.yml");
    let env = repo_file("deploy/env.example");
    let ci = repo_file(".github/workflows/ci.yml");

    assert!(dockerfile.contains("deploy/docker-entrypoint.sh"));
    assert!(entrypoint.contains("require_hex_32 GATEWAY_AUTH_TOKEN"));
    assert!(entrypoint.contains("require_hex_32 SECRETS_MASTER_KEY"));
    assert!(entrypoint.contains("THINCLAW_HOME is not writable"));
    assert!(entrypoint.contains("WORKSPACE_ROOT is not writable"));
    assert!(!entrypoint.contains("printf '%s' \"$gateway_auth_token\""));
    assert!(!entrypoint.contains("printf '%s' \"$secrets_master_key\""));

    for contract in [
        "command: [\"run\", \"--skip-setup-check\"]",
        "ONBOARD_COMPLETED: \"true\"",
        "THINCLAW_HEADLESS: \"true\"",
        "THINCLAW_HOME: /data/.thinclaw",
        "WORKSPACE_ROOT: /workspace",
        "THINCLAW_ALLOW_ENV_MASTER_KEY: \"1\"",
        "DATABASE_BACKEND: libsql",
        "LIBSQL_PATH: /data/thinclaw.db",
        "thinclaw-data:/data",
        "thinclaw-workspace:/workspace",
        "mem_limit: ${THINCLAW_MEMORY_LIMIT:-2g}",
        "pids_limit: ${THINCLAW_PIDS_LIMIT:-512}",
        "driver: json-file",
        "max-size: \"${THINCLAW_LOG_MAX_SIZE:-10m}\"",
    ] {
        assert!(
            compose.contains(contract),
            "missing Compose contract: {contract}"
        );
    }
    assert!(compose.contains("GATEWAY_AUTH_TOKEN:?Generate"));
    assert!(compose.contains("SECRETS_MASTER_KEY:?Generate"));
    assert!(!compose.contains("services:\n  postgres:"));
    assert!(!compose.contains("POSTGRES_PASSWORD:-thinclaw"));

    assert!(env.contains("ONBOARD_COMPLETED=true"));
    assert!(env.contains("THINCLAW_ALLOW_ENV_MASTER_KEY=1"));
    assert!(env.contains("SECRETS_MASTER_KEY=CHANGE_ME_USE_openssl_rand_hex_32"));
    assert!(env.contains("THINCLAW_MEMORY_LIMIT=2g"));

    assert!(ci.contains("docker compose restart thinclaw"));
    assert!(ci.contains("docker compose up -d --force-recreate thinclaw"));
    assert!(ci.contains("compose_persistence_probe"));
    assert!(ci.contains("postgres_compose up -d --force-recreate postgres thinclaw"));
    assert!(ci.contains("postgres_persistence_probe"));
    assert!(ci.contains("[REDACTED]"));
    assert!(!ci.contains("docker-compose.override.yml"));
    assert!(!ci.contains("cat .env > /tmp/compose-env.log"));
}

#[test]
fn postgres_compose_is_explicit_fail_closed_and_persistent() {
    let entrypoint = repo_file("deploy/docker-entrypoint.sh");
    let compose = repo_file("deploy/docker-compose.postgres.yml");
    let docs = repo_file("docs/deploy/docker.md");

    assert!(entrypoint.contains("require_hex_32 POSTGRES_PASSWORD"));
    assert!(compose.contains(
        "pgvector/pgvector:pg15@sha256:a20a57d7aa5217a6af0a391ccf69f4a8512406d6c14be08132f801468cc3cc62"
    ));
    assert!(compose.contains("POSTGRES_PASSWORD:?POSTGRES_PASSWORD must be"));
    assert!(compose.contains("@postgres:5432/"));
    assert!(compose.contains("condition: service_healthy"));
    assert!(compose.contains("thinclaw-postgres:/var/lib/postgresql/data"));
    assert!(compose.contains("pg_isready -U $${POSTGRES_USER} -d $${POSTGRES_DB}"));
    assert!(compose.contains("mem_limit: ${POSTGRES_MEMORY_LIMIT:-2g}"));
    assert!(compose.contains("driver: json-file"));
    assert!(!compose.contains("POSTGRES_PASSWORD:-thinclaw"));
    assert!(!compose.contains("@localhost:5432"));
    assert!(docs.contains("docker-compose.postgres.yml"));
}

#[test]
fn deploy_env_documents_linux_runtime_overrides() {
    let env = repo_file("deploy/env.example");
    for key in [
        "THINCLAW_IMAGE=ghcr.io/rnt56/thinclaw:latest",
        "BROWSER_DOCKER=never",
        "CHROMIUM_IMAGE=chromedp/headless-shell:150.0.7871.125@sha256:7f8ec4782f1b138c30900e65ae53795d5966fbf52168b8fc062843db3e6d5be5",
        "THINCLAW_RUNTIME_PROFILE=pi-os-lite-64",
        "THINCLAW_HEADLESS=true",
        "SCREEN_CAPTURE_ENABLED=false",
        "CAMERA_CAPTURE_ENABLED=false",
        "TALK_MODE_ENABLED=false",
        "LOCATION_ENABLED=false",
        "LOCATION_ALLOW_IP_FALLBACK=false",
        "DESKTOP_AUTONOMY_ENABLED=false",
        "THINCLAW_CAMERA_DEVICE=/dev/video0",
        "THINCLAW_MICROPHONE_DEVICE=default",
        "THINCLAW_MICROPHONE_BACKEND=auto",
    ] {
        assert!(
            env.contains(key),
            "deploy/env.example should mention {}",
            key
        );
    }
}

#[test]
fn pi_os_lite_support_is_documented_and_guarded() {
    let setup = repo_file("deploy/setup.sh");
    let readme = repo_file("README.md");
    let deployment_docs = repo_file("docs/DEPLOYMENT.md");
    let pi_deployment_docs = repo_file("docs/deploy/raspberry-pi-os-lite.md");
    let build_profiles = repo_file("docs/BUILD_PROFILES.md");
    let external_deps = repo_file("docs/EXTERNAL_DEPENDENCIES.md");
    let cli_reference = repo_file("docs/CLI_REFERENCE.md");
    let channel_architecture = repo_file("docs/CHANNEL_ARCHITECTURE.md");
    let cargo_toml = repo_file("Cargo.toml");
    let ci = repo_file(".github/workflows/ci.yml");
    let release = repo_file(".github/workflows/release.yml");

    assert!(setup.contains("is_pi_os_lite_64"));
    assert!(setup.contains("MODE=\"auto\""));
    assert!(setup.contains("THINCLAW_ALLOW_ENV_MASTER_KEY=1"));
    assert!(setup.contains("THINCLAW_RUNTIME_PROFILE=pi-os-lite-64"));
    assert!(setup.contains("THINCLAW_HEADLESS=true"));
    assert!(setup.contains("dotenv_quote"));
    assert!(setup.contains("CHROMIUM_IMAGE=chromedp/headless-shell:150.0.7871.125@sha256:7f8ec4782f1b138c30900e65ae53795d5966fbf52168b8fc062843db3e6d5be5"));
    assert!(readme.contains("docs/DEPLOYMENT.md"));
    assert!(
        pi_deployment_docs.contains("deploy-setup.sh --secrets-stdin --mode native --profile edge"),
        "Pi OS Lite guide should document the supported native edge install"
    );
    assert!(deployment_docs.contains("thinclaw doctor --readiness-profile pi-os-lite-64"));
    assert!(deployment_docs.contains("aarch64-unknown-linux-gnu"));
    assert!(deployment_docs.contains("docker compose pull thinclaw"));
    assert!(deployment_docs.contains("cargo build --release --features full"));
    assert!(deployment_docs.contains("DESKTOP_AUTONOMY_ENABLED=false"));
    assert!(build_profiles.contains("Raspberry Pi OS Lite 64-Bit Builds"));
    assert!(external_deps.contains("pi-os-lite-64"));
    assert!(cli_reference.contains("--profile pi-os-lite-64"));
    assert!(channel_architecture.contains("Raspberry Pi OS Lite 64-bit runs"));
    assert!(cargo_toml.contains("features = [\"full\"]"));
    assert!(ci.contains("ubuntu-24.04-arm"));
    assert!(ci.contains("linux/arm64"));
    assert!(ci.contains("workflow_dispatch"));
    assert!(ci.contains("Verify ARM64 runner"));
    assert!(ci.contains("cargo build --locked --release --features full --bin thinclaw"));
    assert!(ci.contains("./target/release/thinclaw doctor --readiness-profile pi-os-lite-64"));
    assert!(ci.contains("THINCLAW_LINUX_READINESS_OS_RELEASE"));
    assert!(ci.contains("http://127.0.0.1:$port/api/health"));
    assert!(ci.contains("linux-desktop-autonomy-smoke"));
    assert!(ci.contains("gnome-x11"));
    assert!(ci.contains("plasma-kwin-wayland"));
    assert!(ci.contains("openbox-x11"));
    assert!(ci.contains("kwin-wayland"));
    assert!(ci.contains("plasma-workspace"));
    assert!(ci.contains("kde-spectacle"));
    assert!(ci.contains("command -v spectacle"));
    assert!(ci.contains("scripts/ci/linux_desktop_sidecar_smoke.sh"));
    assert!(release.contains("platforms: linux/amd64"));
    assert!(release.contains("platforms: linux/arm64"));
    assert!(release.contains("ghcr.io/${GITHUB_REPOSITORY,,}"));
}

#[test]
fn linux_desktop_sidecar_smoke_covers_expected_sessions() {
    let smoke = repo_file("scripts/ci/linux_desktop_sidecar_smoke.sh");
    assert!(smoke.contains("gnome-x11"));
    assert!(smoke.contains("kde-wayland"));
    assert!(smoke.contains("plasma-kwin-wayland"));
    assert!(smoke.contains("kwin_wayland"));
    assert!(smoke.contains("plasmashell"));
    assert!(smoke.contains("openbox-x11"));
    assert!(smoke.contains("sidecar health"));
    assert!(smoke.contains("sidecar ui"));
    assert!(smoke.contains("sidecar screen"));
    assert!(smoke.contains("assert_health"));
}
