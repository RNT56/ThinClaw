use super::*;

/// Bind the OAuth callback listener on the shared fixed port.
pub async fn find_available_port() -> Result<(TcpListener, u16), AuthError> {
    bind_callback_listener("127.0.0.1", DEFAULT_OAUTH_CALLBACK_PORT).await
}

/// Bind the OAuth callback listener on the requested fixed port.
pub async fn bind_callback_listener(
    host: &str,
    port: u16,
) -> Result<(TcpListener, u16), AuthError> {
    if port == 0 || !is_loopback_host(host) {
        return Err(AuthError::DiscoveryFailed(
            "OAuth callback listener must use a non-zero port on a loopback host".to_string(),
        ));
    }
    let canonical = canonical_loopback_host(host).ok_or_else(|| {
        AuthError::DiscoveryFailed("OAuth callback listener host is invalid".to_string())
    })?;
    let listener = TcpListener::bind(format!("{canonical}:{port}"))
        .await
        .map_err(|_| AuthError::PortUnavailable)?;
    Ok((listener, port))
}

/// Build the authorization URL with all required parameters.
#[allow(clippy::too_many_arguments)]
pub fn build_authorization_url(
    base_url: &str,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    pkce: Option<&PkceChallenge>,
    state: Option<&str>,
    resource: Option<&str>,
    extra_params: &HashMap<String, String>,
) -> Result<String, AuthError> {
    let mut url = reqwest::Url::parse(base_url).map_err(|error| {
        AuthError::DiscoveryFailed(format!("Invalid authorization URL: {error}"))
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || base_url.len() > MAX_AUTHORIZATION_URL_BYTES
        || client_id.is_empty()
        || client_id.len() > MAX_OAUTH_PARAMETER_BYTES
        || client_id.chars().any(char::is_control)
        || redirect_uri.len() > MAX_OAUTH_PARAMETER_BYTES
        || scopes.len() > MAX_OAUTH_METADATA_ITEMS
        || extra_params.len() > MAX_OAUTH_EXTRA_PARAMETERS
    {
        return Err(AuthError::DiscoveryFailed(
            "Authorization endpoint or parameters are malformed or oversized".to_string(),
        ));
    }
    validate_redirect_uri(redirect_uri)?;
    if state.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_OAUTH_PARAMETER_BYTES
            || value.chars().any(char::is_control)
    }) || pkce.is_some_and(|value| {
        value.challenge.is_empty()
            || value.challenge.len() > 128
            || !value
                .challenge
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return Err(AuthError::DiscoveryFailed(
            "OAuth state or PKCE challenge is malformed or oversized".to_string(),
        ));
    }

    const RESERVED: &[&str] = &[
        "client_id",
        "response_type",
        "redirect_uri",
        "scope",
        "code_challenge",
        "code_challenge_method",
        "state",
        "resource",
    ];
    let existing = url
        .query_pairs()
        .filter(|(key, _)| !RESERVED.contains(&key.as_ref()))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    if existing.len() > MAX_OAUTH_EXTRA_PARAMETERS
        || existing.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 128
                || value.len() > MAX_OAUTH_PARAMETER_BYTES
                || key.chars().any(char::is_control)
                || value.chars().any(char::is_control)
        })
    {
        return Err(AuthError::DiscoveryFailed(
            "Authorization endpoint query is malformed or excessive".to_string(),
        ));
    }
    url.set_query(None);
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in existing {
            pairs.append_pair(&key, &value);
        }
        pairs.append_pair("client_id", client_id);
        pairs.append_pair("response_type", "code");
        pairs.append_pair("redirect_uri", redirect_uri);
        if !scopes.is_empty() {
            let scope = scopes.join(" ");
            if scope.len() > MAX_OAUTH_PARAMETER_BYTES || scope.chars().any(char::is_control) {
                return Err(AuthError::DiscoveryFailed(
                    "OAuth scope list is malformed or oversized".to_string(),
                ));
            }
            pairs.append_pair("scope", &scope);
        }
        if let Some(pkce) = pkce {
            pairs.append_pair("code_challenge", &pkce.challenge);
            pairs.append_pair("code_challenge_method", "S256");
        }
        if let Some(state) = state {
            pairs.append_pair("state", state);
        }
        if let Some(resource) = resource {
            if resource.is_empty()
                || resource.len() > MAX_AUTHORIZATION_URL_BYTES
                || resource.chars().any(char::is_control)
            {
                return Err(AuthError::DiscoveryFailed(
                    "OAuth resource parameter is oversized".to_string(),
                ));
            }
            pairs.append_pair("resource", resource);
        }
        for (key, value) in extra_params {
            if RESERVED.contains(&key.as_str()) {
                continue;
            }
            if key.is_empty()
                || key.len() > 128
                || value.len() > MAX_OAUTH_PARAMETER_BYTES
                || key.chars().any(char::is_control)
                || value.chars().any(char::is_control)
            {
                return Err(AuthError::DiscoveryFailed(
                    "OAuth extra parameter is malformed or oversized".to_string(),
                ));
            }
            pairs.append_pair(key, value);
        }
    }
    let result = url.to_string();
    if result.len() > MAX_AUTHORIZATION_URL_BYTES {
        return Err(AuthError::DiscoveryFailed(
            "Authorization URL exceeds the output limit".to_string(),
        ));
    }
    Ok(result)
}

/// Compare two OAuth `state` values in constant time.
///
/// Avoids leaking the expected `state` through a timing side channel when
/// validating the loopback callback. Uses `subtle::ConstantTimeEq`, the same
/// primitive as the WASM tool OAuth flow in `crate::wasm::oauth` and
/// `cli::oauth_defaults`, rather than a hand-rolled comparator.
pub(super) fn oauth_state_matches(expected: &str, received: &str) -> bool {
    use subtle::ConstantTimeEq;
    // `ct_eq` is constant-time only across equal-length inputs; the explicit
    // length check guards the differing-length case (the byte comparison is
    // skipped, but the lengths themselves are not secret).
    expected.len() == received.len() && expected.as_bytes().ct_eq(received.as_bytes()).into()
}

/// Wait for the authorization callback and validate an optional state nonce.
async fn write_callback_response(socket: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let body = if body.len() <= MAX_OAUTH_CALLBACK_HTML_BYTES {
        body
    } else {
        "<html><body>OAuth callback completed.</body></html>"
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(
        OAUTH_CALLBACK_CONNECTION_TIMEOUT,
        socket.write_all(response.as_bytes()),
    )
    .await;
    let _ = socket.shutdown().await;
}

async fn handle_authorization_callback_connection(
    mut socket: tokio::net::TcpStream,
    server_name: String,
    expected_state: Option<String>,
    success_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
    failure_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
) -> Result<Option<AuthorizationCallback>, AuthError> {
    let request_line = tokio::time::timeout(OAUTH_CALLBACK_CONNECTION_TIMEOUT, async {
        let reader = BufReader::new(&mut socket);
        let mut limited = reader.take((MAX_OAUTH_CALLBACK_LINE_BYTES + 1) as u64);
        let mut request_line = String::new();
        let bytes = limited
            .read_line(&mut request_line)
            .await
            .map_err(|error| AuthError::Http(error.to_string()))?;
        Ok::<_, AuthError>((request_line, bytes))
    })
    .await;

    let Ok(Ok((request_line, bytes))) = request_line else {
        write_callback_response(&mut socket, "408 Request Timeout", "").await;
        return Ok(None);
    };
    if bytes == 0
        || request_line.len() > MAX_OAUTH_CALLBACK_LINE_BYTES
        || !request_line.ends_with('\n')
    {
        write_callback_response(&mut socket, "414 URI Too Long", "").await;
        return Ok(None);
    }

    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    };
    if method != "GET" || !matches!(version, "HTTP/1.0" | "HTTP/1.1") || parts.next().is_some() {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" || query.len() > MAX_OAUTH_CALLBACK_LINE_BYTES {
        write_callback_response(&mut socket, "404 Not Found", "").await;
        return Ok(None);
    }

    let mut params = HashMap::new();
    let mut item_count = 0usize;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        item_count = item_count.saturating_add(1);
        if item_count > MAX_OAUTH_EXTRA_PARAMETERS
            || key.is_empty()
            || key.len() > 128
            || value.len() > MAX_OAUTH_PARAMETER_BYTES
            || params
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            write_callback_response(&mut socket, "400 Bad Request", "").await;
            return Ok(None);
        }
    }

    // Validate state before acting on either an error or a code. Otherwise any
    // local process could cancel the real flow with `/callback?error=...`.
    if expected_state.as_deref().is_some_and(|expected| {
        !params
            .get("state")
            .is_some_and(|received| oauth_state_matches(expected, received))
    }) {
        write_callback_response(&mut socket, "400 Bad Request", &failure_html(&server_name)).await;
        return Ok(None);
    }

    if params.contains_key("error") {
        write_callback_response(&mut socket, "400 Bad Request", &failure_html(&server_name)).await;
        return Err(AuthError::AuthorizationDenied);
    }

    let Some(code) = params.remove("code") else {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    };
    if code.is_empty()
        || code.len() > MAX_OAUTH_PARAMETER_BYTES
        || code.chars().any(char::is_control)
    {
        write_callback_response(&mut socket, "400 Bad Request", "").await;
        return Ok(None);
    }

    write_callback_response(&mut socket, "200 OK", &success_html(&server_name)).await;
    Ok(Some(AuthorizationCallback { code }))
}

pub(super) async fn wait_for_authorization_callback(
    listener: TcpListener,
    server_name: &str,
    expected_state: Option<&str>,
    success_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
    failure_html: Arc<dyn Fn(&str) -> String + Send + Sync>,
) -> Result<AuthorizationCallback, AuthError> {
    let expected_state = expected_state.map(str::to_string);
    let server_name = server_name.to_string();
    tokio::time::timeout(Duration::from_secs(300), async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept(), if handlers.len() < MAX_OAUTH_CALLBACK_CONNECTIONS => {
                    let (socket, _) = accepted
                        .map_err(|error| AuthError::Http(error.to_string()))?;
                    handlers.spawn(handle_authorization_callback_connection(
                        socket,
                        server_name.clone(),
                        expected_state.clone(),
                        Arc::clone(&success_html),
                        Arc::clone(&failure_html),
                    ));
                }
                completed = handlers.join_next(), if !handlers.is_empty() => {
                    match completed {
                        Some(Ok(Ok(Some(callback)))) => return Ok(callback),
                        Some(Ok(Err(error))) => return Err(error),
                        Some(Ok(Ok(None))) | Some(Err(_)) | None => {}
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| AuthError::Timeout)?
}
