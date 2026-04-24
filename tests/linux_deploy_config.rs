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
    assert!(compose.contains("GATEWAY_PORT=${GATEWAY_PORT:-3000}"));
    assert!(compose.contains("localhost:$${GATEWAY_PORT:-3000}/api/health"));
    assert!(setup.contains("THINCLAW_PORT=\"${GATEWAY_PORT:-3000}\""));
    assert!(setup.contains("http://localhost:$THINCLAW_PORT/api/health"));
    assert!(deployment_docs.contains("Code-backed default gateway port: `3000`"));
}

#[test]
fn deploy_env_documents_linux_runtime_overrides() {
    let env = repo_file("deploy/env.example");
    for key in [
        "BUILD_FEATURES=full",
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
