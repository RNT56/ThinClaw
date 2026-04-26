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
    assert!(dockerfile.contains("ARG BUILD_FEATURES=full"));
    assert!(compose.contains("ghcr.io/rnt56/thinclaw:latest"));
    assert!(compose.contains("GATEWAY_PORT=${GATEWAY_PORT:-3000}"));
    assert!(compose.contains("localhost:$${GATEWAY_PORT:-3000}/api/health"));
    assert!(setup.contains("THINCLAW_PORT=\"${GATEWAY_PORT:-3000}\""));
    assert!(setup.contains("--mode <auto|native|docker>"));
    assert!(setup.contains("ExecStart=/usr/local/bin/thinclaw run --no-onboard"));
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
    assert!(setup.contains("THINCLAW_FIREWALL_STRICT"));
    assert!(
        !setup.contains("ufw reset"),
        "installer must not reset existing firewall rules"
    );
    assert!(deployment_docs.contains("Code-backed default gateway port: `3000`"));
}

#[test]
fn deploy_env_documents_linux_runtime_overrides() {
    let env = repo_file("deploy/env.example");
    for key in [
        "BUILD_FEATURES=full",
        "THINCLAW_IMAGE=ghcr.io/rnt56/thinclaw:latest",
        "BROWSER_DOCKER=auto",
        "CHROMIUM_IMAGE=chromedp/headless-shell:latest",
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
    assert!(setup.contains("CHROMIUM_IMAGE=chromedp/headless-shell:latest"));
    assert!(readme.contains("deploy-setup.sh --mode native --binary ./thinclaw"));
    assert!(deployment_docs.contains("thinclaw doctor --profile pi-os-lite-64"));
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
    assert!(ci.contains("cargo build --release --features full --bin thinclaw"));
    assert!(ci.contains("./target/release/thinclaw doctor --profile pi-os-lite-64"));
    assert!(ci.contains("THINCLAW_LINUX_READINESS_OS_RELEASE"));
    assert!(ci.contains("http://127.0.0.1:$port/api/health"));
    assert!(ci.contains("linux-desktop-autonomy-smoke"));
    assert!(ci.contains("gnome-x11"));
    assert!(ci.contains("plasma-kwin-wayland"));
    assert!(ci.contains("openbox-x11"));
    assert!(ci.contains("kwin-wayland"));
    assert!(ci.contains("plasma-workspace"));
    assert!(ci.contains("spectacle"));
    assert!(ci.contains("scripts/ci/linux_desktop_sidecar_smoke.sh"));
    assert!(release.contains("linux/amd64,linux/arm64"));
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
