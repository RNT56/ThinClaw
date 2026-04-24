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
            "{path} must not reintroduce the legacy gateway port"
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
    assert!(deployment_docs.contains("Code-backed default gateway port: `3000`"));
}

#[test]
fn deploy_env_documents_linux_runtime_overrides() {
    let env = repo_file("deploy/env.example");
    for key in [
        "BUILD_FEATURES=full",
        "THINCLAW_IMAGE=ghcr.io/rnt56/thinclaw:latest",
        "BROWSER_DOCKER=auto",
        "SCREEN_CAPTURE_ENABLED=false",
        "CAMERA_CAPTURE_ENABLED=false",
        "TALK_MODE_ENABLED=false",
        "LOCATION_ENABLED=false",
        "LOCATION_ALLOW_IP_FALLBACK=false",
        "THINCLAW_CAMERA_DEVICE=/dev/video0",
        "THINCLAW_MICROPHONE_DEVICE=default",
        "THINCLAW_MICROPHONE_BACKEND=auto",
    ] {
        assert!(env.contains(key), "deploy/env.example should mention {key}");
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
    assert!(release.contains("linux/amd64,linux/arm64"));
    assert!(release.contains("ghcr.io/${GITHUB_REPOSITORY,,}"));
}
