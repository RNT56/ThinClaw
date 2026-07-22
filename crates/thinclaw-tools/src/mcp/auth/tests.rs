use super::*;

#[test]
fn test_pkce_challenge_generation() {
    let pkce = PkceChallenge::generate();

    // Verifier should be base64url encoded
    assert!(!pkce.verifier.is_empty());
    assert!(!pkce.verifier.contains('+'));
    assert!(!pkce.verifier.contains('/'));
    assert!(!pkce.verifier.contains('='));

    // Challenge should be different from verifier
    assert_ne!(pkce.verifier, pkce.challenge);

    // Two challenges should be different
    let pkce2 = PkceChallenge::generate();
    assert_ne!(pkce.verifier, pkce2.verifier);
}

#[test]
fn test_oauth_state_matches_constant_time() {
    let state = "a3f1c9d2e4b5a6071829304a5b6c7d8e";
    assert!(oauth_state_matches(state, state));
    assert!(!oauth_state_matches(state, "wrong"));
    assert!(!oauth_state_matches(state, ""));
    // Same length, single-byte difference must be rejected.
    let mut tampered = state.to_string();
    tampered.replace_range(0..1, if state.starts_with('a') { "b" } else { "a" });
    assert!(!oauth_state_matches(state, &tampered));
    // A longer received value with the expected as a prefix must be rejected.
    assert!(!oauth_state_matches(state, &format!("{state}extra")));
}

#[test]
fn test_build_authorization_url() {
    let url = build_authorization_url(
        "https://auth.example.com/authorize",
        "client-123",
        "http://localhost:9876/callback",
        &["read".to_string(), "write".to_string()],
        None,
        None,
        None,
        &HashMap::new(),
    )
    .unwrap();

    assert!(url.starts_with("https://auth.example.com/authorize?"));
    assert!(url.contains("client_id=client-123"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("redirect_uri="));
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(
        parsed
            .query_pairs()
            .find(|(key, _)| key == "scope")
            .map(|(_, value)| value.into_owned()),
        Some("read write".to_string())
    );
}

#[test]
fn test_build_authorization_url_with_pkce() {
    let pkce = PkceChallenge::generate();
    let url = build_authorization_url(
        "https://auth.example.com/authorize",
        "client-123",
        "http://localhost:9876/callback",
        &[],
        Some(&pkce),
        None,
        None,
        &HashMap::new(),
    )
    .unwrap();

    assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
    assert!(url.contains("code_challenge_method=S256"));
}

#[test]
fn test_build_authorization_url_with_extra_params() {
    let mut extra = HashMap::new();
    extra.insert("owner".to_string(), "user".to_string());
    extra.insert("state".to_string(), "abc123".to_string());

    let url = build_authorization_url(
        "https://auth.example.com/authorize",
        "client-123",
        "http://localhost:9876/callback",
        &[],
        None,
        None,
        None,
        &extra,
    )
    .unwrap();

    assert!(url.contains("owner=user"));
    assert!(!url.contains("state=abc123"));
}

#[test]
fn test_build_authorization_url_preserves_generated_state() {
    let mut extra = HashMap::new();
    extra.insert("state".to_string(), "override".to_string());
    extra.insert(
        "resource".to_string(),
        "https://wrong.example.com".to_string(),
    );

    let url = build_authorization_url(
        "https://auth.example.com/authorize",
        "client-123",
        "http://localhost:9876/callback",
        &[],
        None,
        Some("expected-state"),
        Some("https://resource.example.com"),
        &extra,
    )
    .unwrap();

    assert!(url.contains("state=expected-state"));
    assert!(url.contains("resource=https%3A%2F%2Fresource.example.com"));
    assert!(!url.contains("override"));
    assert!(!url.contains("wrong.example.com"));
}

#[test]
fn authorization_url_rejects_unsafe_inputs_and_reserved_query_overrides() {
    let empty = HashMap::new();
    for base in [
        "javascript:alert(1)",
        "https://user:secret@auth.example.com/authorize",
        "https://auth.example.com/authorize#fragment",
    ] {
        assert!(
            build_authorization_url(
                base,
                "client",
                "http://127.0.0.1:9876/callback",
                &[],
                None,
                Some("state"),
                None,
                &empty,
            )
            .is_err()
        );
    }
    assert!(
        build_authorization_url(
            "https://auth.example.com/authorize",
            "client",
            "http://public.example.com/callback",
            &[],
            None,
            Some("state"),
            None,
            &empty,
        )
        .is_err()
    );
    assert!(
        build_authorization_url(
            "https://auth.example.com/authorize",
            "client",
            "https://app.example.com/callback",
            &[],
            None,
            Some("bad\nstate"),
            None,
            &empty,
        )
        .is_err()
    );

    let url = build_authorization_url(
        "https://auth.example.com/authorize?state=attacker&client_id=attacker&tenant=safe",
        "expected-client",
        "https://app.example.com/callback",
        &[],
        None,
        Some("expected-state"),
        None,
        &empty,
    )
    .unwrap();
    let parsed = reqwest::Url::parse(&url).unwrap();
    let pairs = parsed.query_pairs().into_owned().collect::<Vec<_>>();
    assert_eq!(
        pairs
            .iter()
            .filter(|(key, _)| key == "state")
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>(),
        vec!["expected-state"]
    );
    assert_eq!(
        pairs
            .iter()
            .filter(|(key, _)| key == "client_id")
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>(),
        vec!["expected-client"]
    );
    assert!(
        pairs
            .iter()
            .any(|pair| pair == &("tenant".into(), "safe".into()))
    );
}

#[test]
fn token_response_fields_are_strictly_validated() {
    let valid = TokenResponse {
        access_token: "access".to_string(),
        token_type: "bearer".to_string(),
        expires_in: Some(3600),
        refresh_token: Some("refresh".to_string()),
        scope: Some("read write".to_string()),
    };
    assert_eq!(
        access_token_from_response(valid).unwrap().token_type,
        "Bearer"
    );

    let wrong_type = TokenResponse {
        access_token: "access".to_string(),
        token_type: "mac".to_string(),
        expires_in: None,
        refresh_token: None,
        scope: None,
    };
    assert!(access_token_from_response(wrong_type).is_err());
    let injected = TokenResponse {
        access_token: "access\r\nX-Injected: yes".to_string(),
        token_type: "Bearer".to_string(),
        expires_in: None,
        refresh_token: None,
        scope: None,
    };
    assert!(access_token_from_response(injected).is_err());

    let implausible_expiry = TokenResponse {
        access_token: "access".to_string(),
        token_type: "Bearer".to_string(),
        expires_in: Some(MAX_OAUTH_TOKEN_LIFETIME_SECS + 1),
        refresh_token: None,
        scope: None,
    };
    assert!(access_token_from_response(implausible_expiry).is_err());
}

#[tokio::test]
async fn stored_access_tokens_preserve_expiry_and_expired_rows_are_not_authenticated() {
    use secrecy::SecretString;
    use thinclaw_secrets::{InMemorySecretsStore, SecretsCrypto};

    let crypto = Arc::new(
        SecretsCrypto::new(SecretString::from("mcp-test-master-key-32-bytes-long!!")).unwrap(),
    );
    let secrets: Arc<dyn SecretsStore + Send + Sync> = Arc::new(InMemorySecretsStore::new(crypto));
    let config = McpServerConfig::new("expiry-test", "https://mcp.example.com");
    let before = chrono::Utc::now();
    let token = AccessToken {
        access_token: "access".to_string(),
        token_type: "Bearer".to_string(),
        expires_in: Some(120),
        refresh_token: Some("refresh".to_string()),
        scope: None,
    };

    store_tokens(&secrets, "user", &config, &token)
        .await
        .unwrap();
    let stored = secrets
        .get("user", &config.token_secret_name())
        .await
        .unwrap();
    let expires_at = stored.expires_at.expect("expiry must be persisted");
    assert!(expires_at >= before + chrono::Duration::seconds(119));
    assert!(expires_at <= chrono::Utc::now() + chrono::Duration::seconds(121));
    assert!(is_authenticated(&config, &secrets, "user").await);

    secrets
        .create(
            "user",
            CreateSecretParams::new(config.token_secret_name(), "expired")
                .with_expiry(chrono::Utc::now() - chrono::Duration::seconds(1)),
        )
        .await
        .unwrap();
    assert!(!is_authenticated(&config, &secrets, "user").await);
}

#[tokio::test]
async fn callback_slowloris_and_wrong_state_do_not_cancel_valid_flow() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let callback = tokio::spawn(wait_for_authorization_callback(
        listener,
        "test",
        Some("expected-state"),
        Arc::new(|_| "success".to_string()),
        Arc::new(|_| "failure".to_string()),
    ));

    // One peer deliberately never finishes its request line. A valid
    // callback must still be processed by another bounded handler.
    let _slow = tokio::net::TcpStream::connect(address).await.unwrap();

    let mut wrong = tokio::net::TcpStream::connect(address).await.unwrap();
    wrong
        .write_all(
            b"GET /callback?error=denied&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await
        .unwrap();
    let mut wrong_response = Vec::new();
    wrong.read_to_end(&mut wrong_response).await.unwrap();
    assert!(wrong_response.starts_with(b"HTTP/1.1 400"));

    let mut valid = tokio::net::TcpStream::connect(address).await.unwrap();
    valid
        .write_all(
            b"GET /callback?code=valid-code&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), callback)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.code, "valid-code");
}
