# WS-07 — Gateway Remote-Access Hardening

| | |
|---|---|
| **Priority** | P1 (7.1–7.6) · P2 (rest) |
| **Effort** | M |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01; WS-05 (CSP + no inline handlers, for 7.2/7.5) · decision **D7** |
| **Findings** | H3 (GW-1, GW-2, GW-3), GW-4, GW-6 … GW-10 |
| **Canonical docs** | `src/NETWORK_SECURITY.md`, `docs/DEPLOYMENT.md`, `docs/deploy/remote-access.md`, `docs/MOBILE_SECURITY.md`, `.env.example`; regenerate `clients/openapi/thinclaw-gateway.openapi.json` if any DTO changes (`cargo run --example export-openapi -- generate`) |

## Goal

The default token mode is already sound. Make the opt-in *ambient* modes (Tailscale identity, trusted proxy) — which authenticate by network position rather than a secret — resistant to browser-borne attacks (CSWSH, CSRF, DNS rebinding), and stop the TLS listener from widening exposure.

## Tasks

- [ ] **7.1 — Enforce WebSocket Origin (GW-1).**
  `crates/thinclaw-gateway/src/web/ports.rs:~449-471` `validate_websocket_origin` accepts a missing Origin and any http(s) Origin; `chat_ws_handler` (`src/channels/web/handlers/chat.rs:~495-498`) relies on it. Require Origin ∈ {CORS allowlist ∪ configured advertised origins} for browser-originated upgrades; allow a missing Origin only for non-browser clients authenticated by bearer/device token, never for ambient auth. Make `src/NETWORK_SECURITY.md:~189-200` match.

- [ ] **7.2 — CSRF defense when auth is ambient (GW-1).**
  With Tailscale/proxy identity, cross-site `no-cors` POSTs reach body-less state-changing routes (`/api/gateway/restart`, `/api/extensions/{name}/activate`, `/api/extensions/{name}/remove`, `/api/autonomy/rollback`, `/api/costs/reset`). For `GatewayAuthSource::{Tailscale, TrustedProxy}`, require `Origin`/`Sec-Fetch-Site: same-origin` (or a per-session CSRF token) on every non-GET.

- [ ] **7.3 — Host allowlist + Serve provenance (GW-2).**
  No Host check exists anywhere in the gateway (DNS rebinding). `crates/thinclaw-gateway/src/web/auth.rs:~358` trusts `Tailscale-User-Login` from **any loopback peer** when `allow_serve_proxy`. Validate `Host` against configured names; require a Serve-specific marker (e.g. per-boot secret header configured on `tailscale serve`, or a Unix socket) before honoring Serve identity headers.

- [ ] **7.4 — Trusted-proxy mode (GW-2, D7).**
  `auth.rs:~259-262` treats an empty `TRUSTED_PROXY_IPS` as "trust loopback", and `:~487` grants `GatewayRole::Admin` to any header value. Managed cloudflared/ngrok tunnels connect over loopback (`src/tunnel/cloudflare.rs:~36, ~47`), so header-name knowledge = Admin from the internet. Refuse to start trusted-proxy mode with an empty IP list while a tunnel is active; map the trusted-proxy identity to a configured role (default not Admin). Document `TRUSTED_PROXY_HEADER` / `TRUSTED_PROXY_IPS` in `docs/DEPLOYMENT.md` and `.env.example`.

- [ ] **7.5 — Serve job-written files safely (GW-3).**
  `src/channels/web/handlers/projects.rs:~27-52` serves `~/.thinclaw/projects/<id>/*` with an extension-guessed MIME (`text/html`, `image/svg+xml`), no CSP, no `Content-Disposition`; the directory is bind-mounted writable into job containers (`src/orchestrator/job_manager.rs:~1207-1209`); the UI links it (`app.js:~2377, ~5238`). Serve from a separate origin/port, or with `Content-Security-Policy: sandbox` + `Content-Disposition: attachment` for active types. (Also: "Browse Files" 401s in token mode — WS-11.)

- [ ] **7.6 — TLS listener bind (GW-4).**
  `src/channels/web/tls.rs:~411` binds `0.0.0.0` unconditionally; `gateway-tls` is in the default `light` profile, `GATEWAY_TLS=Auto`, and the listener auto-starts on the first admin `pair/start` (`handlers/devices.rs:~377`) or at boot if any device is paired — serving the whole router (incl. public `pair/complete`, `/hooks/*`, `/metrics`) regardless of `GATEWAY_HOST=127.0.0.1`. Default the TLS bind to `GATEWAY_HOST`; add `GATEWAY_TLS_HOST` for explicit LAN exposure; log the bind. Fix `docs/DEPLOYMENT.md:~29` and `src/NETWORK_SECURITY.md:~42`.

- [ ] **7.7 — Logs are admin surface (GW-6).**
  `/api/logs/events`, `/api/logs/recent` (`src/channels/web/server.rs:~1242-1243`) need only `ReadState`, stream DEBUG for all principals, scrubbed only by a regex `LeakDetector` on `message` (`log_layer.rs:~69-76`). Add `/api/logs` to `ADMIN_SURFACE_PREFIXES`; apply the passwordless 60-s re-auth guard used by chat SSE/WS (`chat.rs:~417`, `ws.rs:~108`) to logs SSE.

- [ ] **7.8 — Tailscale whois hardening (GW-7).**
  `auth.rs:~405` gates on source IP ∈ 100.64/10 but not on the local (tailnet) interface; unmapped peers spawn a `tailscale whois` per request with no negative cache and 4 permits (`try_acquire` → 401). Check local address is a Tailscale address; add a short negative cache.

- [ ] **7.9 — Per-IP pairing lockout (GW-8).**
  `crates/thinclaw-gateway/src/web/devices/pairing.rs:~657`: 10 failures / 5 min locks out everyone. Make lockout per source (with a high global ceiling), reachable unauthenticated via 7.6.

- [ ] **7.10 — Routine webhook replay (GW-9).**
  `routines.rs:~584-608` HMACs only the body. Add timestamp + nonce to the signed material with a window; escape backticks when fencing payload in the fallback path (`handlers/routines.rs:~728-733`).

- [ ] **7.11 — Optional `/metrics` auth (GW-10).**
  `server.rs:~992` is unauthenticated by design when Prometheus is enabled (exposes provider/model/tool/channel names, cost). Add an optional bearer for scrapes; keep the documented default.

- [ ] **7.12 — Small correctness fixes.**
  `query_token` (`auth.rs:~640`) compares the raw query value — URL-decode it (tokens with `+`, `/`, `%` break SSE auth today). `GatewayChannel::new` (`src/channels/web/mod.rs:~114-142`) promises to print a generated token but never prints or persists it — print once to the local console (never logs) or persist to the secret store. Fix `/api/health` response shape in `src/NETWORK_SECURITY.md`.

- [ ] **7.13 — Pending items from `docs/MOBILE_SECURITY.md`.**
  Device-auth failure tarpit (§8 item 2), schedule `sweep_inactive` (item 6), and plan proof-of-possession for the registered Secure Enclave key (documented as v2).

## Verification (workstream exit)

- Unit/integration tests for each auth source: cross-origin WS upgrade rejected; cross-site POST rejected under ambient auth; foreign `Host` rejected; loopback request carrying Tailscale headers without the Serve marker rejected; trusted-proxy mode refuses empty IP list with an active tunnel.
- `cargo run --example export-openapi -- generate` shows no unintended drift.
