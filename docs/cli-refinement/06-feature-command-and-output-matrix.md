# Cross-Cutting Feature, Command, and Output Contract

> **Temporary execution input.** This is the exhaustive leaf/profile/output contract. It is not a sixth workstream. CR-01…CR-05 own implementation and proof; files 07 and 08 separately govern exact runtime identities/channels and setup pages/state. Delete all three matrices after canonical generated documentation contains the shipped facts.

## 1. Audit basis and contract notation

Audited source anchors include `Cargo.toml`, `.github/workflows/{ci,release}.yml`, `src/cli/**/*.rs`, `src/async_main{.rs,/command_dispatch.rs}`, `src/app.rs`, `src/config/mod.rs`, `crates/thinclaw-{app,config,settings,db,agent,channels,channels-core,gateway,tools}/`, the gateway route tree, and the current slash/TUI adapters. The baseline has no repository-local `AGENTS.md`.

Every user leaf below has all of these decisions:

- **Owner:** `LOCAL` (local files/process), `DB` (configured durable store), `GW` (authenticated running runtime), `OS` (platform service/process manager), or `EXT` (external executable/service). Multiple owners mean an adapter crosses those boundaries. Every `M`/`D` leaf additionally has the generated D-54 execution policy; an owner label never authorizes bypassing an active coordinator.
- **Output:** `R` versioned record, `Q` bounded record sequence, `S` live stream, `A` raw artifact, or `I` interactive UI. `A→file` means artifact bytes/text go to `--out`; the status record may use `--output-format`. `A→stdout` means stdout is only the artifact.
- **Risk:** `RO`, `M` mutation, `D` destructive/high-impact, `SECRET`, `COST`, or `INTERNAL`.
- **Availability:** all valid host builds unless a feature/OS/external dependency is named. Availability is still represented by the independent capability dimensions in CR-02.10.
- **Proof:** the black-box/contract family that must contain a concrete test for every listed leaf. A grouped row is acceptable only when the test table enumerates each leaf.

No leaf is implemented directly from this table. Canonical and compatibility parsers convert to the typed request owned by the cited task.

## 2. Supported build and release profiles

| Profile/feature set | Compiled facts today | Required CLI/runtime end state | Required proof |
|---|---|---|---|
| `edge` (`--no-default-features --features edge`) | libSQL only; no PostgreSQL, Docker, Wasmtime, chromiumoxide, Nostr, document extraction, TLS/mDNS, ACP, tunnel, voice, Bedrock, or bundled WASM. | Fresh default is libSQL. REPL, TUI, service, setup, direct DB CLI, gateway, external-browser CLI, and Comfy admin remain present. WASM artifacts can be managed but execution says `not_compiled`. | `PROF-edge`: compile/clippy/test-no-run; root/category/leaf help; completion; static capability; backend-default; service parser. |
| `light` / default | `edge` + PostgreSQL, Docker sandbox, Wasmtime, HTML/document tools, timezones, gateway TLS, mDNS. No ACP/tunnel/chromiumoxide/Nostr/voice/Bedrock/bundled payload. | Dual-backend 0.17/0.18 compatibility fallback is PostgreSQL only when setup/config has not selected; 0.19 requires explicit selection. In 0.17 the aggregate drops empty `timezones` with no behavior change. REPL/TUI/service remain present. Agent browser reports not compiled; `dev browser` independently probes external browsers. | `PROF-light`: existing CI row plus CLI/help/completion/capability snapshots and channel-disable integration. |
| `desktop` | libSQL, HTML/document tools, `repl`, timezones; no Docker/Wasmtime/PostgreSQL/browser/Nostr/TLS/mDNS. | libSQL default; host CLI contracts remain truthful. In 0.17 the aggregate drops empty `repl`/`timezones` with no surface change. Desktop-owned microphone/browser capabilities are distinct from headless Cargo features. | `PROF-desktop`: Linux/Windows compile row, backend default, help/capability, and desktop shared-contract gate. |
| `full` | light + ACP, `web-gateway` compatibility flag, `repl`, tunnel, chromiumoxide browser, Nostr. Voice/Bedrock/bundled WASM are intentionally absent. | Production full headless behavior; in 0.17 the aggregate and inherited light set drop empty `web-gateway`/`repl`/`timezones`. No claim that voice/Bedrock/bundled artifacts are included. | `PROF-full`: CI row, full help/capability, gateway/channel/tool assembly, release smoke. |
| minimal libSQL | `--no-default-features --features libsql`. | Same backend rule as edge; commands remain parseable where their implementation is base code, with dependency reasons instead of false health. | `PROF-min-libsql`. |
| minimal PostgreSQL | `--no-default-features --features postgres`. | PostgreSQL default; libSQL-specific operations report not compiled and never fall back. | `PROF-min-pg`, including service-backed DB contract CI. |
| all features | All optional features, including voice, Bedrock, bundled WASM, integration/schema test flags. Requires ALSA on Linux plus WASI target/tools. | User capability output omits test-only `integration`/`schema-divergence`; reports real voice/Bedrock/bundled-WASM facts. | `PROF-all`: all-target clippy/test plus help/capability and bundled-WASM checks. |
| empty compatibility deltas | Base supported set plus each of `repl`, `web-gateway`, or `timezones` separately and together. `repl` still controls unrelated production code at the audited baseline; `web-gateway` still gates `tests/host_runtime_smoke.rs` despite having no runtime/dependency effect; only `timezones` has no source consumer. | In 0.17 all three are non-aggregate empty aliases: move the gateway smoke under the actual always-available gateway/profile gate, then make dependency graph, compiled tests, help, completion, and static capability metadata identical to the base set. Delete declarations in 0.19. | `PROF-empty-compat`: Cargo metadata/test-discovery diff plus byte-identical generated help/completion/capability fixtures for each delta. |
| cargo-dist release | `features=["full"]` for macOS aarch64/x86_64, Linux GNU/musl aarch64/x86_64, Windows x86_64. | Packaged behavior matches `full`; Windows service dispatcher remains hidden; no voice/Bedrock/bundled-WASM overclaim. | `REL-full`: packaged `--version`, help, status, backend/config failure, hidden-token smoke per target. |
| explicit edge release | `edge` on Linux GNU/musl aarch64/x86_64. | Packaged behavior matches `edge`, defaults to libSQL, and includes service management. | `REL-edge`: packaged help/status/service/backend smoke and checksum/static-link gates. |

Individual features are capability inputs, not product profiles:

| Feature | User-visible meaning | Must not mean |
|---|---|---|
| `wasm-runtime` | Local WASM tool/channel execution engine is compiled. | An installed artifact is registered, exposed, or healthy. |
| `bundled-wasm` | Release embeds catalog artifacts and implies `wasm-runtime`. | Every embedded extension is installed/enabled. |
| `browser` | Agent chromiumoxide browser backend is compiled. | `dev browser` availability; that utility probes external Chrome-family executables. |
| `docker-sandbox` | Docker job runtime and hidden worker/bridge entrypoints are compiled. | Docker daemon/image readiness. |
| `nostr` | Nostr channel/tool code is compiled. | Keys/owner/relay configuration or health. |
| `gateway-tls`, `mdns`, `tunnel` | Optional listener/discovery/tunnel implementations are compiled. | Enabled, reachable, or correctly configured. |
| `voice` | Headless audio-capture runtime is compiled. | Enabled; `THINCLAW_VOICE_WAKE` and dependencies/assets remain separate facts. |
| `bedrock` | Bedrock embedding provider is compiled. | AWS authentication/readiness. |
| `acp` | ACP integration and `thinclaw-acp` auxiliary binary can be built. | ACP is active in the main runtime. |
| `html-to-markdown`, `document-extraction` | Corresponding content tools are compiled. | Every external/media input is supported or healthy. |
| `repl`, `web-gateway`, `timezones` | Empty deprecated compatibility aliases outside every aggregate in 0.17; each controls no code/dependency/metadata and is removed in 0.19. | Service/gateway availability, local REPL/TUI availability, timezone support, or any runtime capability. |
| `integration`, `schema-divergence` | Test compilation/execution gates. | Runtime capability output. |

## 3. Root, setup, status, and agents

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| bare `thinclaw`; `run` | same | LOCAL+DB+EXT / I / M+COST | One runtime path; runtime flags scoped; local REPL exists in every host profile; only selected channels/listeners activate. | CR-01.3/01.4, CR-02.13 / `CLI-run`, `PROF-runtime-surfaces` |
| `tui` | same | LOCAL+DB+EXT / I / M+COST | Same agent/runtime services as REPL with typed bootstrap/history/events/approvals; no shell escape. | CR-03 / `TUI-runtime-contract` |
| global `-m/--message` | `ask TEXT` | LOCAL+DB+EXT / R / M+COST | One local turn; hidden old flag only in 0.17; explicit agent/model selection uses runtime policy. | CR-01.4 / `CLI-ask` |
| `message send` | `send TEXT` | GW / R / M+COST | Authenticated injection through shared client; return accepted ID; never confused with local `ask`. | CR-01.5 / `GW-send` |
| `onboard [--profile PROFILE]` | `setup [--mode quick\|advanced] [--profile PRESET] [--run] [--input FILE] [--dry-run\|--yes]` | LOCAL+DB+SECRET / I or R / M | Draft/review/apply; six retained setup presets are distinct from readiness/build profiles; Quick visibly defaults balanced; custom maps to Advanced/No preset during compatibility. Headless input is 1-MiB versioned/secret-free; dry-run returns a plan digest and non-TTY apply requires matching digest+`--yes`. Explicit setup exits; interactive `--run` opts into REPL. | CR-01.6/01.10 / `SETUP-entry`, `SETUP-apply`, `SETUP-profile-contract`, `SETUP-headless-contract` |
| `onboard --guide[=TOPIC]` | `setup --mode advanced`; `setup edit TOPIC` | LOCAL+DB+SECRET / I / M | Advanced is comprehensive; explicit edit is one topic plus verify/review and exits. Hidden old guide maps to edit with legacy continuation. | CR-01.10 / `SETUP-advanced`, `SETUP-edit-topics` |
| `onboard --channels-only` | `setup edit channels` | LOCAL+DB+SECRET / I / M | One channel-catalog-backed focused edit; canonical exits; no plaintext secret or per-step write. | CR-01.10, CR-02.7 / `SETUP-edit-channels` |
| root `reset` | `setup reset` | DB encrypted secrets/source records+LOCAL+OS credential store / R / D+SECRET | Exact repeatable scope, dry-run parity, stopped-runtime lease, confirmation, typed partial failures. | CR-01.9 / `SETUP-reset-contract` |
| `status [--profile PROFILE]` | `status [--readiness-profile PROFILE] [--live]` | LOCAL+DB; optional GW/EXT / R / RO | Static default performs no network and exits 0 unless collection fails. Readiness and build profiles are separate fields. One sorted revision; live bounded checks exit 3 on required unhealthy facts. | CR-02.10/02.11 / `HEALTH-status-contract`, `HEALTH-readiness-profile` |
| no terminal tool-inventory leaf | `status [--readiness-profile PROFILE] tools [NAME] [--all] [independent fact filters] [--live]` | LOCAL+DB; optional EXT / Q,R / RO | Registered population by default; exact name resolves the full catalog; `--all`/`--match` include catalog and installed-dynamic descriptors. Independent compile/config/register/dependency/exposure/approval/health filters; no synthetic linear state. Same revision/population as `/tools`. | CR-02.10/02.11 / `CAP-tools-contract` |
| `doctor [--profile PROFILE]` | `doctor [--readiness-profile PROFILE]` | LOCAL+DB+EXT / R / RO | Neutral values `server\|remote\|desktop\|pi-os-lite-64\|all-features`; active bounded diagnosis with no `--live`; complete report exit 0/3, operational exit 1, no billable generation. Hidden old flag/value aliases through 0.18. | CR-02.11 / `HEALTH-doctor-contract`, `HEALTH-readiness-profile` |
| Clap `-h/--help`, `-V/--version`, and generated subcommand help | `-h/--help`; `-V/--version`; `help [PATH...] [--all]` | LOCAL / A→stdout parser information / RO | Deterministic raw text even beside `--output-format`; one metadata source; empty/path/all contracts are exact, canonical-only, state/network-free, width-stable, and exit 0. Unknown/hidden help path is usage exit 2; `--all` conflicts with a path. | CR-04.5 / `HELP-root`, `HELP-path`, `HELP-all`, `VERSION-contract` |
| `agents list` | same | DB / Q / RO | Durable registry list; empty envelope is `[]`. | CR-02.2 / `AGENT-list` |
| `agents show ID` | same | DB / R / RO | Durable record; missing ID exits 1. | CR-02.2 / `AGENT-show` |
| `agents add` | same | DB+LOCAL workspace / R / M | Shared create validation, persistence, workspace seeding; success after commit. | CR-02.2 / `AGENT-add` |
| no CLI update leaf | `agents update ID` | DB+LOCAL workspace / R / M | Expose existing registry update service; validate model/tools/skills/channels/profile. | CR-02.2 / `AGENT-update` |
| `agents set-default ID` | same | DB / R / M | Transactional convenience over update; one durable default. | CR-02.2 / `AGENT-default` |
| `agents remove ID` | same | DB+LOCAL / R / D | Confirmation/`--yes` for noninteractive use; safe default/in-use conflict or explicit reassignment/force policy. | CR-02.2 / `AGENT-remove` |

The runtime's five agent-management tools (create/list/update/remove/message) are all accounted for: the first four back the admin leaves; `message_agent` remains an active, identity-scoped inter-agent tool and is not misrepresented as registry CRUD.

## 4. Configuration, secrets, and models

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `config init [-o/--output]` | `config init [--out] [--force] [--yes]` | LOCAL / A→file+R / M | Selected/default TOML only; regular-file/symlink checks; refuse overwrite by default; validate before atomic publish. | CR-01.2, CR-02.6 / `CFG-init` |
| `config list` | same | two-phase resolver / Q / RO+SECRET | Values and source/phase metadata; sensitive values absent, not prefix-leaked. | CR-02.6 / `CFG-list` |
| `config get KEY` | same | resolver / R / RO+SECRET | Exact typed value/source; secret paths rejected. | CR-02.6 / `CFG-get` |
| `config set KEY VALUE` | `config set KEY VALUE [--source database\|toml]` | DB or LOCAL / R / M | Schema-class default routing, full validation, atomic/transactional commit, secret/runtime-only rejection. | CR-02.6 / `CFG-set` |
| `config reset KEY` | `config reset KEY [--source database\|toml]` | DB or LOCAL / R / M | Remove only selected source override; distinct from full reset. | CR-02.6 / `CFG-reset` |
| `config path` | same | LOCAL / R / RO | Report Phase A/Phase B active paths/order/existence without credentials. | CR-02.6 / `CFG-path` |
| `secrets status`; `secrets list` | `config secrets status\|list`; `config secrets show ID` | DB encrypted store+source registry+OS master-key store / R,Q / RO+SECRET | Source ID/label/kind/purpose/in-use/configured/resolvable metadata only; remote output never exposes source locations or values. | CR-02.6 / `SECRET-read`, `SECRET-source-records` |
| `secrets set NAME [--value VALUE] [--provider P] [--user U]` | `config secrets set LABEL [--from-stdin\|--from-env VAR\|--from-file FILE] --for PURPOSE... [--principal ID] [--replace]` | DB encrypted store+source registry; optional ENV/LOCAL / R / M+SECRET | Default masked TTY; explicit bounded stdin or validated env/file reference; generated opaque ID and exact purposes. Replacement preserves ID/increments revision and requires confirmation/`--replace`. Remove unsafe `--value` immediately; hidden provider/user flag adapters only through 0.18. | CR-02.6 / `SECRET-set`, `SECRET-no-argv-value`, `SECRET-source-kinds` |
| `secrets delete NAME` | `config secrets delete ID [--yes]` | DB encrypted store+source registry / R / D+SECRET | Confirm; exact missing ID; refuse in-use source until consumers are rebound/disabled; no partial false success. | CR-02.6 / `SECRET-delete` |
| `secrets rotate-master` | `config secrets rotate-master [--yes]` | DB encrypted store+OS master-key store / R / D+SECRET | Confirm; atomic encrypted-record re-encryption with rollback; env/file/OS source records remain not applicable and never enter mixed key state. | CR-02.6 / `SECRET-rotate` |
| `models list`; `models info MODEL` | `config models list\|info` | LOCAL; optional discovery cache / Q,R / RO | Shared presentation; no paid calls. | CR-01.2, CR-02.16 / `MODEL-read` |
| `models test MODEL` | `config models test MODEL` | EXT / R / COST | One explicit bounded minimal live request; help labels remote/cost; redacted transport. | CR-02.16 / `MODEL-test` |
| `models verify [--discovery-only]` | `config models verify [--chat-probe] [--yes]` | EXT / Q / RO or COST | Discovery default; chat is explicit/confirmed; old inverted flag hidden through 0.18. | CR-02.16 / `MODEL-verify` |
| `models sync [-o/--output]` | `config models sync [--out]` | EXT+LOCAL / A→file+R / M | Discovery/catalog ingestion only; atomic/refuse overwrite policy; no chat. | CR-01.2, CR-02.16 / `MODEL-sync` |

## 5. Runtime, logs, updates, and completion

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `gateway start`; `gateway reload` | `runtime web start\|reload` | LOCAL+GW process / R or I foreground / M | Honest full headless-runtime/web-ingress wording; operation lock/private PID ownership; bounded startup; no token URL and no unused instance/bearer token in child argv/environment. | CR-01.5/01.8, CR-02.12 / `WEB-lifecycle`, `WEB-no-child-token` |
| `gateway stop` | `runtime web stop` | LOCAL process / R / M | Stop only owned PID instance; idempotent missing-state result. | CR-02.12 / `WEB-stop` |
| `gateway status` | `runtime web status` | LOCAL+GW / R / RO | Static owned-process record plus a bounded local health probe when an owned PID exists; otherwise health is `not_probed`. Never equate configured with running. | CR-02.10 / `WEB-status` |
| `gateway access [--show-token]` | `runtime web access [--reveal-token]` | LOCAL+GW / R human / SECRET | Redacted default; only confirmed TTY reveal; machine/pipe/quiet forbidden; hidden alias through 0.18. | CR-02.15 / `WEB-access` |
| `service install`; `service uninstall` | `runtime service install\|uninstall` | OS / R / D | Available independent of `repl`; platform unit ownership; confirm/`--yes`; no implicit binary install/removal. | CR-02.13 / `SERVICE-install` |
| `service start`; `service stop` | `runtime service start\|stop` | OS / R / M | Bounded platform action with real status/error. | CR-02.13 / `SERVICE-control` |
| `service status` | `runtime service status` | OS / R / RO | Platform state, unit path, PID/last failure where available. | CR-02.13 / `SERVICE-status` |
| `logs tail` | `runtime logs tail [--follow]` | LOCAL / S / RO | Human or JSONL only when following; bounded initial lines; interrupt 130. | CR-01.2 / `LOG-tail` |
| `logs search`; `logs show`; `logs levels` | `runtime logs search\|show\|levels` | LOCAL / Q,R / RO | Presentation `--format` removed; bounded files/time ranges and terminal sanitization. | CR-01.2 / `LOG-read` |
| `update check`; `update info` | `runtime update check\|info` | EXT+LOCAL / R / RO | Signed metadata/checksum facts; bounded/no redirects outside policy. | CR-01.5 / `UPDATE-read` |
| `update install`; `update rollback` | `runtime update install\|rollback [--yes]` | EXT+LOCAL / R / D | Signature/checksum, exact target/backup, atomic replacement/recovery, confirmation, no service surprise. | CR-01.1 / `UPDATE-mutate` |
| `completion --shell SHELL` | same | LOCAL / A→stdout / RO | Raw script only; no envelope/banner/warning; canonical supported paths only. | CR-01.2/01.8, CR-04.5 / `COMPLETION-canonical-profiles` |

## 6. Extensions: channels, WASM tools, registry, and MCP

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `channels list`; `channels info NAME` | `extensions channels list\|info` | LOCAL config+optional runtime snapshot / Q,R / RO | Independent compile/config/register/dependency/exposure/health facts; platform/feature reasons. | CR-02.7/02.10 / `CHANNEL-read` |
| `channels validate` | `extensions channels check-config` | LOCAL / Q / RO | Zero network; hidden old alias; never claim connected/healthy. | CR-02.7 / `CHANNEL-static` |
| no leaf | `extensions channels probe [NAME\|--all]` | EXT+optional GW / Q / RO | Bounded safe probes; unsupported is explicit; no test message side effect. | CR-02.7 / `CHANNEL-probe` |
| `tool install`; `tool remove` | `extensions tools install\|remove` | DB+LOCAL+EXT registry / R / M/D | Preserve signature/hash/source/auth/sandbox policy; confirm overwrite/remove; artifact-installed does not imply executable. The WASM-specific adapters pass `kind=wasm-tool` to the shared typed lifecycle where semantics overlap. | CR-01.2, CR-02.13/02.18 / `WASM-mutate`, `EXT-selector` |
| `tool list`; `tool info` | `extensions tools list\|info` | DB+LOCAL / Q,R / RO | Include artifact, compile, registration, dependency, exposure, health state. | CR-02.10/02.13 / `WASM-read` |
| `tool auth`; gateway/WS/chat raw-token completion | `extensions tools auth NAME [--kind KIND] [--credential-source ID\|local secure-source flags]` | DB+secret-source service+EXT / R/I / M+SECRET | Owner/kind/purpose-bound expiring auth session. Server-side OAuth/PKCE or local masked/stdin/env/file creation; remote completion is ID-only. Delete HTTP/WS/free-form-chat token submission immediately. Redacted output/audit; authentication returns `next_action=activate` and never auto-activates. | CR-01.5, CR-02.6/02.18 / `WASM-auth`, `EXT-auth-single-effect`, `EXT-auth-no-token-dto`, `EXT-auth-source-binding` |
| no generic activation leaf | `extensions activate NAME [--kind KIND]` | GW+EXT manager / R / M | `GatewayClient` reaches the live runtime's shared four-kind selector; optional kind resolves cross-kind ambiguity; return activated identities/revision/readiness; typed auth challenge, no string matching, no invented deactivate. Stopped runtime is a non-mutating error. | CR-02.17/02.18 / `EXT-activate`, `EXT-activate-stopped` |
| `registry list`; `registry search`; `registry info` | `extensions registry list\|search\|info` | LOCAL catalog+optional EXT / Q,R / RO | Deterministic filters, bounded search, provenance/signature metadata. | CR-01.2 / `REG-read` |
| `registry install`; `registry install-defaults` | same canonical group | DB+LOCAL+EXT / R,Q / M | Shared extension manager; bulk defaults report each result; overwrite/bulk confirmation. | CR-02.10 / `REG-install` |
| `registry remove` | same canonical group | DB+LOCAL / R / D | Confirm and report dependents/in-use conflict. | CR-02.10 / `REG-remove` |
| `registry validate` | `extensions registry check` | LOCAL+optional EXT / Q / RO | Static manifest/signature/artifact validation; old leaf hidden. | CR-04.2 / `REG-check` |
| `mcp server add ... --env KEY=VALUE`; `remove\|list\|show` | `extensions mcp server add ... [--env KEY=VALUE] [--secret-env KEY=SOURCE_ID]`; `remove\|list\|show` | DB / R,Q / M/RO | Durable config; `--env` accepts validated non-secret entries only, secret env uses manifest-namespaced purpose bindings, and credential-bearing URL/command args are rejected. Remove confirms. Reads expose no source locator/value. | CR-02.6 / `MCP-server-crud`, `MCP-secret-env`, `MCP-no-secret-argv` |
| `mcp server auth\|test` | same | DB+EXT+secret-source service / R/I / SECRET+RO | Auth uses the shared OAuth/session/source-ID or local secure-create contract; no raw token DTO/chat input and no activation side effect. `test` is live, labeled, bounded, and resolves the exact binding. | CR-01.5, CR-02.6/02.18 / `MCP-server-live`, `EXT-auth-no-token-dto` |
| `mcp server toggle --enable\|--disable` | `extensions mcp server enable\|disable NAME` | DB / R / M | Cleaner explicit verbs; hidden old toggle through 0.18. | CR-04 / `MCP-server-toggle` |
| `mcp resource list\|read\|templates` | same canonical group | DB+EXT / Q,R / RO | Requires configured server; bounded payload; resource content is record/domain data, not terminal decoration. | CR-01.5 / `MCP-resource` |
| `mcp prompt list\|get` | same canonical group | DB+EXT / Q,R / RO | Bounded arguments/content and typed protocol errors. | CR-01.5 / `MCP-prompt` |
| `mcp root list\|grant\|revoke` | same canonical group | DB / Q,R / RO or D | Canonicalize/contain paths; grant/revoke confirmation and audit; no symlink escape. | CR-02.6 / `MCP-root` |
| `mcp log show\|set` | same canonical group | DB/LOCAL runtime config / R / RO or M | Typed log level; shared resolver; no handler-local presentation flag. | CR-02.6 / `MCP-log` |

## 7. Extensions: all 17 skill lifecycle operations

| Existing runtime tool | Canonical CLI leaf | Owner / output / risk | Fixed policy | Proof |
|---|---|---|---|---|
| `skill_list` | `extensions skills list` | LOCAL registry / Q / RO | Source/trust/provenance/status parity. | `SKILL-list` |
| `skill_search` | `extensions skills search QUERY` | LOCAL+EXT catalog/hub / Q / RO | Bounded source-specific search and safe remote errors. | `SKILL-search` |
| `skill_inspect` | `extensions skills inspect NAME` | LOCAL registry+quarantine / R / RO | Files/provenance/findings; bounded/sanitized. | `SKILL-inspect` |
| `skill_read` | `extensions skills read NAME` | LOCAL in-memory registry / A→stdout or R / RO | Full instruction content is an explicit artifact/read; no path escape. | `SKILL-read` |
| `skill_check` | `extensions skills check (--path\|--url\|--stdin)` | LOCAL+optional EXT / R / RO | Exactly one source; scan without install; bounded HTTPS/file/stdin. | `SKILL-check` |
| `skill_install` | `extensions skills install NAME` | LOCAL+EXT+quarantine / R / M | Scan/provenance/source policy; overwrite/risk confirmation. | `SKILL-install` |
| `skill_update` | `extensions skills update NAME` | LOCAL+EXT+quarantine / R / M | Recorded provenance lock required; scan/risk confirmation. | `SKILL-update` |
| `skill_audit` | `extensions skills audit [NAME]` | LOCAL / Q / RO | Scanner only, no mutation. | `SKILL-audit` |
| `skill_snapshot` | `extensions skills snapshot [--out]` | LOCAL / A→file+R / M | Deterministic inventory/hash artifact, atomic/refuse overwrite. | `SKILL-snapshot` |
| `skill_publish` | `extensions skills publish NAME --target-repo …` | LOCAL+EXT GitHub / R / M | Dry-run default; remote write only `--execute --yes`, matching tap, scan, draft PR. | `SKILL-publish` |
| `skill_tap_list` | `extensions skills taps list` | DB+optional EXT health / Q / RO | Stored sources; live health only when requested/bounded. | `SKILL-tap-list` |
| `skill_tap_add` | `extensions skills taps add OWNER/REPO` | DB / R / M | Owner/repo/path/branch validation; replace confirmation. | `SKILL-tap-add` |
| `skill_tap_remove` | `extensions skills taps remove OWNER/REPO` | DB / R / D | Exact tap identity and confirmation. | `SKILL-tap-remove` |
| `skill_tap_refresh` | `extensions skills taps refresh [OWNER/REPO]` | DB+EXT / Q / M | Bounded refresh; preserve old valid state on failure. | `SKILL-tap-refresh` |
| `skill_remove` | `extensions skills remove NAME` | LOCAL / R / D | Source ownership/in-use validation and confirmation. | `SKILL-remove` |
| `skill_reload` | `extensions skills reload [NAME\|--all]` | LOCAL runtime registry / Q,R / M | Shared re-discovery; no duplicate registry; all is bounded. | `SKILL-reload` |
| `skill_trust_promote` | `extensions skills trust NAME --to installed\|trusted` | LOCAL / R / M+SECURITY | Only allowed ceilings; explicit confirmation/audit; never bypass scan. | `SKILL-trust` |

There is no durable skill enable/disable or general quarantine-release operation in the audited service. Do not invent those leaves. Quarantine findings and risk approval stay inside check/install/update/inspect/trust policy.

## 8. Automation: routines, jobs, and repository projects

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `cron list`; `cron runs`; `cron lint` | `automation routines list\|runs\|lint` | DB / Q / RO | Typed routine/run records; lint is static. | CR-02.4 / `ROUTINE-read` |
| `cron add`; `cron edit` | `automation routines add\|edit` | DB / R / M | Shared routine schema/validation, transactional commit. | CR-02.4 / `ROUTINE-write` |
| `cron remove` | `automation routines remove [--yes]` | DB / R / D | Confirm; handle active run/foreign owner precisely. | CR-02.4 / `ROUTINE-remove` |
| fake `cron trigger` | `automation routines trigger ID` | GW / R / M+COST | Create real run once and return `routine_id`, `run_id`, status; no `--wait`. | CR-02.4 / `ROUTINE-trigger` |
| no CLI | `automation jobs list\|summary\|show` | GW / Q,R / RO | Owned direct+sandbox projections; server-side filters/cursor/limits. | CR-02.5 / `JOB-read` |
| no CLI | `automation jobs cancel\|restart ID` | GW / R / D | Confirmation, ownership masking, state conflicts. | CR-02.5 / `JOB-control` |
| no CLI | `automation jobs prompt ID TEXT` | GW / R / M | Only interactive/accepting jobs; return queued result. | CR-02.5 / `JOB-prompt` |
| no CLI | `automation jobs events ID` | GW / Q / RO | Bounded `limit`/exclusive `after`; no follow/reconnect claim. | CR-02.5 / `JOB-events` |
| no CLI | `automation jobs files list ID [PATH]` | GW / Q / RO | Bounded contained directory listing. | CR-02.5 / `JOB-files-list` |
| no CLI | `automation jobs files read ID PATH [--out]` | GW / A→stdout/file / RO | Bounded UTF-8 only; binary explicit error; exact contained text. | CR-02.5 / `JOB-files-read` |
| `repo-projects list\|show\|status\|events` | `automation projects list\|show\|status\|events` | DB; optional EXT readiness / Q,R / RO | Durable project/event/readiness facts; bounded events. | CR-04 / `PROJECT-read` |
| `repo-projects setup`; `set-credential [--value VALUE]` | `automation projects setup`; `automation projects set-credential SLOT [--from-stdin\|--from-env VAR\|--from-file FILE] [--replace]` | DB+secret-source service / R/I / M+SECRET | Four exact source-bound slots; default masked TTY; no non-TTY implicit input. Remove `--value` immediately. Agent/gateway bind `{slot, secret_source_id}` only; no credential text in tool transcript/API/settings/output. | CR-02.6 / `PROJECT-setup`, `PROJECT-secret-binding`, `PROJECT-no-argv-value` |
| `repo-projects create`; `enroll` | `automation projects create\|enroll` | DB / R / M | Validate repo/workspace/write-mode policy and persist. | CR-04 / `PROJECT-create` |
| `repo-projects repos`; `connect` | `automation projects repos\|connect` | DB+EXT GitHub+SECRET / Q,R / RO/M | Bounded authenticated discovery; explicit selected/all connection plan. | CR-01.5 / `PROJECT-connect` |
| `repo-projects start\|pause\|resume` | `automation projects start\|pause\|resume` | DB+runtime observer / R / M | Durable transition accepted; runtime availability stated. | CR-04 / `PROJECT-control` |
| `repo-projects cancel` | `automation projects cancel [--yes]` | DB+runtime observer / R / D | Confirm; durable terminal transition. | CR-04 / `PROJECT-cancel` |

## 9. Labs experiments

All experiment commands are expert/optional under `labs experiments`; code is base-compiled today, while execution also depends on settings, DB, gateway, provider credentials, runner leases, and optional Docker/cloud backends.

| Current leaf set | Canonical leaf set | Owner / risk | Required end state | Proof |
|---|---|---|---|---|
| `experiments enable\|disable` | `labs experiments enable\|disable` | DB / M | Shared config resolver, typed outcome. | `EXP-toggle` |
| `projects list\|show` | same nested path | DB / RO | Bounded typed records. | `EXP-project-read` |
| `projects create\|update` | same nested path | DB / M | Full metric/path/runner/promotion/autonomy validation. | `EXP-project-write` |
| `projects delete` | same nested path + `--yes` | DB / D | Confirmation and dependent campaign conflict. | `EXP-project-delete` |
| `runners list\|show\|validate` | same nested path | DB+optional EXT / RO | Static validation separate from live provider probe. | `EXP-runner-read` |
| `runners create\|update` with inline `*_json`/secret names | same nested path with bounded `--*-file` inputs and `--secret-env ENV=SOURCE_ID` | DB / M+SECRET refs | Typed backend/profile schema; generic files are secret-free and reject credential-shaped keys/values; purpose-authorized opaque bindings only. Hidden legacy JSON flags through 0.18 accept only the same validated non-secret schema. | `EXP-runner-write`, `EXP-runner-secret-binding` |
| `runners delete` | same nested path + `--yes` | DB / D | Confirm and active lease/campaign conflict. | `EXP-runner-delete` |
| `campaigns list\|show` | same nested path | DB / RO | Typed campaign/lease/run facts. | `EXP-campaign-read` |
| `campaigns start\|pause\|resume\|cancel` | same nested path; `start/resume [--auth-out FILE] [--yes]` | DB+GW/runner / M/D+COST+conditional SECRET artifact | Shared client; cancel confirmation; return IDs/state only. Manual remote start/resume requires local private auth artifact before mutation; auto-advance pauses at `awaiting_secure_delivery`. Managed launch keeps the bearer internal and rejects `--auth-out`. | `EXP-campaign-control`, `EXP-runner-secret-transport` |
| `campaigns promote`; token-bearing `reissue-lease` response/bootstrap | `campaigns promote`; `campaigns reissue-lease ID [--auth-out FILE] [--yes]` | DB+GW+EXT / D+SECRET artifact | Explicit confirmation/policy audit. Reissue revokes once and returns metadata only; managed secret delivery is internal. Manual delivery is local-only to an atomic private one-use artifact; no bearer/bootstrap command in argv/API/UI/logs. | `EXP-campaign-admin`, `EXP-runner-secret-transport`, `EXP-no-token-output` |
| `opportunities list` | same nested path | GW / RO | Bounded authenticated list. | `EXP-opportunities` |
| `targets list` | same nested path | GW / RO | Bounded authenticated list using the safe typed metadata projection; invalid legacy metadata is `legacy_blocked` with field names only, never echoed values. | `EXP-target-read`, `EXP-target-metadata` |
| `targets link\|update` with inline `--metadata-json`; `delete` | link/update with bounded `--metadata-file FILE`; delete with `--yes` | GW / M/D | Shared typed client and IDs. One ten-kind metadata catalog, exact non-secret annotations bounds, unknown/credential-shaped rejection, and no silent non-object-to-empty coercion. Hidden old JSON alias through 0.18 uses the same parser. Blocked legacy rows require explicit replacement. Delete confirms. | `EXP-target-write`, `EXP-target-metadata` |
| `providers list` | same nested path | GW / RO | Capability/auth state without secrets. | `EXP-provider-read` |
| `providers connect --payload-json` (currently sends a wrapper that does not match the path DTO) | `providers connect PROVIDER --credential-source ID` | GW+EXT / M+SECRET | Exact typed GatewayClient request with authorized `experiment:provider:<backend>:api` binding; remove generic payload immediately; no plaintext API key or mismatched wrapper. | `EXP-provider-connect`, `EXP-provider-dto-parity` |
| `providers validate --payload-json` | `providers validate PROVIDER` | GW+EXT / RO+SECRET | No request body; resolve the stored binding server-side and run one bounded live validation. | `EXP-provider-validate`, `EXP-provider-dto-parity` |
| `providers launch-test --payload-json` | `providers launch-test PROVIDER [--yes]` | GW+EXT / COST+M | No generic request body; resolve stored binding server-side, require explicit cost warning/confirmation, bound the test, and return job/lease ID. | `EXP-provider-test`, `EXP-provider-dto-parity` |

The hidden `experiment-runner` remains an orchestrator lease entrypoint, not a public substitute for these leaves.

## 10. Durable data

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `memory search` | `data memory search` | DB+embedding provider / Q / RO | Rank/score/path/document/chunk/citation and bounded excerpt; paid embeddings only under existing explicit provider policy. | CR-02.8 / `MEM-search` |
| `memory read`; `memory tree`; `memory status` | same nested path | DB+LOCAL workspace / R,Q / RO | Search IDs/citations interoperate; bounded/sanitized output. | CR-02.8 / `MEM-read` |
| `memory write` | same nested path | DB+LOCAL workspace / R / M | Authorized workspace path, atomic durable indexing result. | CR-02.8 / `MEM-write` |
| no CLI delete | `data memory delete PATH [--dry-run] [--yes]` | DB+LOCAL workspace / R / D | Shared memory policy, protected identity files, containment/index cleanup, exact dry-run parity, confirmation. | CR-02.18 / `MEM-delete` |
| ephemeral `sessions list\|show` | `data conversations list\|show` | DB / Q,R / RO | Durable identity-scoped conversations/messages with cursors. | CR-02.3 / `CONV-read` |
| no old search/delete | `data conversations search\|delete` | DB / Q,R / RO/D | Store-side bounded search; delete dry-run/confirmation. | CR-02.3 / `CONV-search-delete` |
| `sessions export` | `data conversations export --artifact-format markdown\|json [--out]` | DB / A→stdout/file / RO | Exact ordered durable message artifact, no wrapper collision. | CR-02.3 / `CONV-export` |
| `sessions prune` | `data conversations prune` | DB / Q,R / D | Store-side scope, dry-run default, `--yes`, same selection in execution. | CR-02.3 / `CONV-prune` |
| `backup export [--passphrase VALUE]` | `data backup export [--passphrase-file FILE]` | LOCAL+DB+EXT `pg_dump` / A→file+R / M+SECRET | Remove plaintext argument immediately; masked TTY/private file/env precedence, typed partiality, `--require-database`, atomic/refuse overwrite. PostgreSQL uses a private one-use `PGPASSFILE`, never inherited `PGPASSWORD`/credential URL. | CR-02.14 / `BACKUP-export`, `BACKUP-no-argv-passphrase`, `BACKUP-no-child-password` |
| `backup inspect [--passphrase VALUE]` | `data backup inspect FILE [--passphrase-file FILE]` | LOCAL / R / RO+SECRET | Remove plaintext argument immediately; validate/decrypt bounded manifest without mutation. | CR-02.14 / `BACKUP-inspect`, `BACKUP-no-argv-passphrase` |
| `backup import [--passphrase VALUE]` | `data backup import FILE [--passphrase-file FILE]` | LOCAL+DB / R / D+SECRET | Remove plaintext argument immediately; dry-run default, exclusive lease, containment, confirmation, backend-honest restore. | CR-02.14 / `BACKUP-import`, `BACKUP-no-argv-passphrase` |
| `trajectory export` | `data trajectories export --artifact-format jsonl\|json\|sft\|dpo [--out]` | LOCAL archive+optional DB / A→stdout/file / RO/M file | Preserve real formats, manifest rules, bounded records, atomic artifact; rename path option. | CR-01.2/CR-04 / `TRAJ-export` |
| `trajectory stats` | `data trajectories stats` | LOCAL archive+optional DB / R / RO | Actual leaf retained; no fictitious inspect command. | CR-04 / `TRAJ-stats` |

### Learning administration from agent tools and the existing gateway API

| Existing capability / current CLI | Canonical leaf | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `learning_status` + gateway status; no CLI | `data learning status [--recent N] [--live]` | DB; optional GW+EXT / R / RO | Static default reads settings/counts/last probe with zero network; bounded live probes all eight providers concurrently; principal scope. | CR-02.19 / `LEARN-status-static`, `LEARN-status-live` |
| `learning_history` + gateway history/candidate/version/feedback/rollback/proposal lists; no CLI | `data learning history KIND [filters] [--limit N] [--cursor TOKEN]` | DB / Q / RO | Exact kind-specific filters; stable keyset cursor for one kind; `all` returns per-kind cursors; no unusable `has_more`. | CR-02.19 / `LEARN-history-kinds`, `LEARN-cursor-contract` |
| `learning_outcomes` + gateway outcome list/detail; no CLI | `data learning outcomes list\|show` | DB / Q,R / RO | Principal/actor-scoped contracts and observations; typed filters; stable cursor; exact not-found. | CR-02.19 / `LEARN-outcome-read` |
| gateway outcome review; no agent/CLI leaf | `data learning outcomes review ID --decision confirm\|dismiss\|requeue [--verdict …] [--yes]` | DB / R / D | Exact decision/verdict matrix, confirmation, audit, outcome-ledger persistence; no provider call. | CR-02.19 / `LEARN-outcome-review` |
| gateway evaluate-now; no agent/CLI leaf | `data learning outcomes evaluate-now [--yes]` | GW+LLM / R / M+COST | Running runtime only; disclose/bound billable model requests; confirmation; return processed count and evaluation IDs/failures. | CR-02.19 / `LEARN-outcome-evaluate` |
| `learning_feedback` + gateway feedback submit; no CLI | `data learning feedback submit TARGET_TYPE TARGET_ID --verdict VERDICT [--note] [--metadata-file FILE]` | DB / R / M | Shared validation/audit; bounded regular JSON-object metadata; no secret/settings channel. Feedback history is the `feedback` history kind. | CR-02.19 / `LEARN-feedback` |
| gateway proposal list + `learning_proposal_review`; no CLI | `data learning proposals list\|show\|review` | DB; approval uses GW+LOCAL+optional EXT Git / Q,R / RO/D | List cursor/detail; review effect preview. Approve is dry-run by default noninteractively and confirmed before contained bundle/Git/publish effects; reject is audited. | CR-02.19 / `LEARN-proposal-contract` |
| gateway rollback list/POST; no agent/CLI leaf | `data learning rollbacks record ARTIFACT_TYPE ARTIFACT_NAME --reason TEXT [--version ID] [--metadata-file FILE] [--yes]` | DB / R / M | Truthfully append a rollback ledger observation only; result says `artifact_restored=false`; validate artifact/version/principal. History uses kind `rollbacks`; no fictitious apply. | CR-02.19 / `LEARN-rollback-record` |
| `external_memory_status` + gateway provider health; no CLI | `data learning external-memory status [--live]` | DB; optional GW+EXT / R / RO | Static configuration/last probe separate from bounded live health; agent exposure remains hidden by default. | CR-02.19 / `LEARN-ext-status` |
| `external_memory_setup`; no CLI | `data learning external-memory configure PROVIDER [typed options] [purpose-scoped SecretSourceId options] [--enabled BOOL]` | DB+secret-source metadata / R / M+SECRET | Exact eight-provider schema catalog; zero-network configure-only operation; independent configured/enabled/active/scope-safe facts. Reject inline/source-location/generic secret config and migrate every legacy secret-like field to bindings. | CR-02.19 / `LEARN-ext-schema-all-providers`, `LEARN-ext-secret`, `LEARN-ext-configure-static` |
| activation currently bundled into `external_memory_setup`; no CLI | `data learning external-memory activate PROVIDER [--yes]` | GW+EXT runtime / R / M+SECRET | Separate confirmed live-runtime action; require configured, enabled, credential-resolvable, healthy, and strict subject scope; return active provider and snapshot revision. Letta is denied until scope-safe. | CR-02.19 / `LEARN-ext-activate`, `LEARN-ext-scope-policy` |
| `external_memory_off`; no CLI | `data learning external-memory deactivate [--yes]` | DB; live shutdown GW+EXT / R / D | Confirm; clear only persisted active selection, preserve provider configuration/enabled state, and report persisted plus live shutdown outcomes without recall/export. Agent `external_memory_off` maps here. | CR-02.19 / `LEARN-ext-deactivate` |

`prompt_manage`, `skill_manage`, `external_memory_recall`, and `external_memory_export` remain agent-only. The first two require contextual evidence/versioned approval; skill lifecycle is already exposed safely under `extensions skills`; recall/export are operational provider actions, not missing administration.

## 11. Access: identities, senders, and devices

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `identity list\|show` | `access identities list\|show` | DB / Q,R / RO | Principal-scoped durable actor/endpoint records. | CR-04 / `IDENTITY-read` |
| `identity create\|rename\|set-preferred-channel` | same under `access identities` | DB / R / M | Shared identity registry validation/commit. | CR-04 / `IDENTITY-write` |
| `identity link\|unlink` | same under `access identities` | DB / R / M/D | Endpoint uniqueness/ownership; unlink confirmation when last/preferred endpoint. | CR-04 / `IDENTITY-link` |
| `pairing list`; `pairing blocked` | `access senders list\|blocked` | DB / Q / RO | Preserve pending and blocked lists as distinct durable states; pending output includes request ID but never its pairing code. | CR-04 / `SENDER-read`, `SENDER-no-code-output` |
| `pairing approve CHANNEL CODE`; `block\|unblock` | `access senders approve CHANNEL REQUEST_ID`; `block\|unblock` | DB / R / M | Approve by exact pending request ID with ownership/channel scope and audit. Remove positional-code parsing immediately; codes are absent from argv, errors, JSON, and history. | CR-01.8/CR-04 / `SENDER-write`, `SENDER-no-code-argv` |
| `devices pair` | `access devices pair` | GW / R/I / M+SECRET | Pairing code/QR is ephemeral secret: deliberate human display only, never logs/snapshots/machine history. | CR-01.5 / `DEVICE-pair` |
| `devices list` | `access devices list` | GW / Q / RO | Shared authenticated client, ownership-scoped. | CR-01.5 / `DEVICE-list` |
| `devices rename` | `access devices rename` | GW / R / M | Accepted durable result. | CR-01.5 / `DEVICE-rename` |
| `devices revoke` | `access devices revoke [--yes]` | GW / R / D | Confirmation; return revoked ID/status. | CR-01.5 / `DEVICE-revoke` |

## 12. Media and developer utilities

| Current 0.16 leaf(s) | Canonical leaf(s) | Owner / output / risk | Required end state | Task / proof |
|---|---|---|---|---|
| `comfy health`; `hardware-check`; `check-deps` | `media comfy health\|hardware-check\|check-deps` | EXT Comfy/executables / R,Q / RO | Bounded probes; distinguish external dependency, config, and health. | CR-02.10 / `COMFY-check` |
| `comfy setup` | `media comfy setup [--yes]` | EXT installer+LOCAL / R / D | Explicit confirmation before installer/download; checksum/source policy. | CR-04 / `COMFY-setup` |
| `comfy launch`; `comfy stop` | `media comfy launch\|stop` | EXT process / R / M | Owned process/PID, bounded readiness/shutdown. | CR-04 / `COMFY-control` |
| `comfy list-workflows` | same nested path | EXT+LOCAL / Q / RO | Bounded sanitized workflow metadata. | CR-04 / `COMFY-list` |
| `comfy generate` | same nested path | EXT / R / M+COST compute | Explicit generation parameters, bounded poll, artifact IDs/paths, no false success. | CR-04 / `COMFY-generate` |
| `browser check` | `dev browser check` | EXT Chrome-family executable / R / RO | Probe binary/version independently of Cargo `browser`. | CR-02.13 / `BROWSER-check` |
| `browser open [--format]` | `dev browser open [--artifact-format text\|html]` | EXT subprocess/network / A→stdout or R / RO | Bounded process/output/URL scheme; presentation and domain format do not collide. | CR-01.2/02.13 / `BROWSER-open` |
| `browser screenshot [-o/--output]` | `dev browser screenshot [--out]` | EXT subprocess/network+LOCAL / A→file+R / M file | Atomic/refuse overwrite, dimensions bounded, no auto-open. | CR-01.2 / `BROWSER-shot` |
| `browser links` | `dev browser links` | EXT subprocess/network / Q / RO | Bounded/sanitized URLs and link text. | CR-02.13 / `BROWSER-links` |

## 13. Internal process entrypoints

| Token | Compile gate | Disposition | Proof |
|---|---|---|---|
| `worker`, `claude-bridge`, `codex-bridge` | `docker-sandbox` | Keep executable for orchestrator; add Clap `hide=true`; absent from help/all-help/completion/reference. | `INTERNAL-docker-help`, `INTERNAL-docker-invocation` |
| `network-relay` | `docker-sandbox` | Already hidden; preserve fixed-target validation/auth. | `INTERNAL-relay` |
| `experiment-runner --lease-id … --token VALUE [--workspace-root]` | base today | Hide immediately and remove `--token`; consume a capped versioned auth envelope from exactly one of private stdin/file/managed secret delivery. Preserve a contained validated workspace root. No token-bearing generated bootstrap command, response, OpenAPI/client type, provider template, UI, argv, or log. | `INTERNAL-experiment`, `EXP-runner-secret-transport`, `EXP-no-token-output` |
| `autonomy-shadow-canary` | base | Already hidden; preserve manifest validation. | `INTERNAL-canary` |
| `__windows-service` | Windows and currently `repl` | Keep hidden, decouple from `repl`, restrict to Windows SCM invocation. | `INTERNAL-windows`, `PROF-profile-matrix` |
| `thinclaw-acp` auxiliary binary | `acp` | Keep separate protocol binary; not a root subcommand. | `INTERNAL-acp-binary` |
| `thinclaw-shell-scan` auxiliary binary | base | Keep developer/safety binary; not a root subcommand. | `INTERNAL-shell-scan` |

## 14. Runtime agent capability inventory and final-snapshot timing

The CLI must compare itself to the actual agent registry rather than a remembered total. File 07 is the exhaustive identity authority: 124 statically named IDs across conditional paths plus dynamic MCP/WASM/user/native sources, with every name and CLI disposition listed. The grouped rows below are assembly checkpoints only and cannot substitute for that identity ledger.

| Capability origin | Actual wiring observed | Availability inputs | Snapshot requirement |
|---|---|---|---|
| Core built-ins | Registered during `AppBuilder` core tool setup. | Workspace/profile/policy and underlying deps. | Stable descriptor per registered tool, not just a count. |
| Filesystem/dev/process/execute-code/search | Registered according to workspace mode, base/working directory, safety scanner, Docker/sandbox backend. | Workspace scope, `docker-sandbox`, Docker readiness, network policy. | Explain disabled-by-project/sandbox policy versus dependency missing. |
| Memory/session search | Registered when DB/workspace memory services exist. | DB, embeddings, authorized workspace. | Distinguish DB configured/healthy and tool exposed. |
| Vision/MoA/TTS/Comfy/media | Registered from LLM/media/config readiness; some are conditional. | Model support, secrets, external Comfy, OS/platform. | No optimistic “available” from registration alone. |
| Extension manager | Six agent tools: search/install/auth/activate/list/remove. | Extension manager, source/auth/policy. | Provenance and mutation approval policy. |
| Skills | Seventeen tools enumerated in section 7. | Registry/catalog/remote hub/quarantine/DB settings. | Exact 17-name parity fixture when service constructed. |
| Learning/external memory | Prompt/skill management plus learning status/outcomes/history/feedback and external-memory operations, conditionally registered. | DB, workspace, skill registry, provider readiness. | Origin and each conditional reason; no hard-coded total. |
| Repository projects | Project create/plan/status/pause/resume/enroll/setup/approve/credential/repos/connect variants, conditional on stores/secrets. | DB, secrets, GitHub configuration. | Describe registered subset and credential/dependency state. |
| Desktop/local tools | Screen/camera/talk/location/Apple Mail/desktop autonomy are platform/config/opt-in dependent. | OS, desktop policy, permission, headless blocker. | Present exact platform and policy reason; never advertise on unsupported OS. |
| Jobs | Create/list/status/cancel plus events/prompt when their ports exist. Registered before and again with scheduler binding. | DB/context manager, Docker manager, event store, prompt queue, scheduler. | Final descriptor reconciles replacement/late binding; no double count. |
| Send-message/Nostr | Send-message registered after channels; Nostr action tool only with feature/runtime. | Channel registration/exposure and Nostr feature/config. | Channel-specific availability and policy. |
| Subagents | Three late tools: spawn/list/cancel. | Executor/DB/routine finalization and parent grants. | Included only after executor creation; preserve grant policy. |
| LLM/advisor | Model selection/list registered late; advisor added/reconciled dynamically. | Provider/model and advisor readiness. | Dynamic health/registration update without stale boot count. |
| Persistent agents | Five runtime agent tools registered after DB registry load. | DB registry/router and authorization. | Included after load/registration, with CLI parity for CRUD. |
| Routines | Five routine tools registered through shared store/engine lifecycle. | DB, engine, scheduler/runtime. | Definition versus execution readiness separated. |
| WASM/MCP/native plugins | Dynamic installed/loaded registrations. | Compile feature, artifacts, validation, auth, runtime registration, profile exposure. | Descriptor provenance and reason at every independent dimension. |

Collection point: complete the prepare/seal sequence in file 07 after final scheduler-bound jobs, channels/send-message, skills/learning/projects/media, subagents, LLM/advisor initial state, persistent agents, routines, WASM/MCP/native plugins, and collision reconciliation. Boot/TUI/status consume startup revision `N`; hot changes publish `N+1`. Early counts may be logged for diagnostics only and never labeled final.

### 14.1 Desktop capability-consumer contract

The desktop does not add static agent-tool IDs, but it consumes and mutates the same runtime contracts. These committed baseline facts must not regress during the CLI/runtime refactor.

| Current desktop fact | Required end state | Proof |
|---|---|---|
| `thinclaw_spawn_session` plus desktop child-session list/update are embedded-runtime operations and now reject a selected remote profile with a typed `LocalOnly` gate. | Preserve the pre-mutation gate. Do not silently spawn locally, map it to the agent's subagent tools, or advertise remote support. A later remote implementation requires a separate authenticated profile-targeted contract. | `DESKTOP-subagent-route-boundary` |
| Fleet status now leaves `current_task` absent when no authoritative task identity exists, but capabilities are still projected heuristically from local config/remote gateway fields. | Consume the versioned capability snapshot or lossless projection; preserve `null`/unknown task state and never synthesize “Ready,” connection-count tasks, or capability claims. | `DESKTOP-fleet-capability-projection` |
| Local channel schema returns non-password values and explicitly reports `secret_binding_available=false`; password fields are omitted. | Keep credential inputs disabled until the target runtime accepts authorized opaque secret-source IDs. Once wired, return only source metadata/configured state; never persist or return a raw channel credential. | `DESKTOP-channel-secret-binding` |
| Desktop llama.cpp/vLLM sidecars put generated loopback bearer tokens in `--api-key`; MLX uses `THINCLAW_MLX_API_KEY`; managed STT copies a token into process-global bridge env; internal endpoint DTO shapes retain token fields even where renderer commands fill them with an empty string. | Route every sidecar through the process-launch descriptor/clean environment contract. Deliver ephemeral auth through the backend's proven inherited-pipe/private-file adapter and in-memory endpoint registry, remove managed global-env injection and token fields from all serializable/Specta/frontend/runtime-contract DTOs, and fail `auth_transport_unsupported` instead of falling back to argv/env. | `DESKTOP-sidecar-auth-transport`, `DESKTOP-sidecar-no-token-dto`, `PROCESS-launch-manifest` |
| The compiled monolithic desktop runtime builder injects Gmail/gateway/LLM secrets into generic `BRIDGE_VARS`; four tracked extraction modules beside it are not declared and duplicate active assembly logic. | Finish one compiled modular builder, delete duplicate/orphan regions, use typed non-secret `DesktopRuntimeInputs`, and resolve purpose-bound credentials only inside owning services. Keep local/remote route gates and final capability identities unchanged. | `DESKTOP-builder-module-topology`, `DESKTOP-no-secret-bridge-map`, `DESKTOP-builder-runtime-parity` |

## 15. REPL/TUI/agent-message command surface ledger

The current shared registry contains `/help`, `/status`, `/context`, `/model` (`/models`), `/rollback`, `/rewind`, `/plan`, `/version`, `/tools`, `/debug`, `/ping`, `/undo`, `/redo`, `/compress` (`/compact`), `/clear`, `/interrupt` (`/stop`), `/new`, `/thread new`, `/thread <id>`, `/resume <id>`, `/identity`, `/personality` (`/vibe`), `/skin`, `/memory`, `/heartbeat`, `/summarize` (`/summary`), `/suggest`, `/skills`, `/restart`, and `/quit` (`/exit`, `/shutdown`). The TUI also hard-codes `/back`, `/close`, `/dismiss`, `/top`, `/bottom`, `/reset`, `/think`, `/job`, `/cancel`, `/list`, `/thread`, `/resume`, `/cls`, plus `!command`; the REPL owns another static list and help table.

Target routing classes:

| Class | Commands | Target |
|---|---|---|
| Local client presentation/navigation | `/help`, `/debug`, `/skin`, `/status`, `/tools`, `/back`, `/close`, `/dismiss`, `/top`, `/bottom`, `/cls`, `/quit` aliases | Typed local REPL/TUI route. `/debug` changes only client diagnostic visibility. `/status` and `/tools [NAME] [--all]` render the latest whole capability revision; unsupported client routes are omitted by surface metadata. |
| Typed conversation/thread actions | `/undo`, `/redo`, `/clear`, `/compress`, `/interrupt`, `/new`, `/thread …`, `/resume …`, `/rewind`, `/plan`, `/restart` | Explicit local or forwarded route with conversation identity and capability/runtime predicate. No display-only placeholder may be executable. |
| Shared system/query actions | `/context`, `/model`, `/version`, `/ping`, `/identity`, `/personality`, `/memory`, `/heartbeat`, `/summarize`, `/suggest`, `/skills` | Registry-declared route per REPL/TUI/agent-message surface; remote agent-message routes require declared authorization. `/tools` has a separately declared authorized agent-message route even though it is locally rendered on terminal clients. |
| Job commands | Canonical `/job create DESCRIPTION`, `/job list [FILTER]`, `/job status [ID]`, `/job cancel ID`, `/job help ID`; legacy `/job DESCRIPTION`, `/create`, `/list`, `/jobs`, `/status ID`, `/cancel ID`, and `/help ID` | Register explicit typed `MessageIntent` routes and argument schemas. Keep legacy forms hidden through 0.18, resolving bare `/status` and `/help` to their system commands without ambiguity. They may not live only in an array. |
| Remove | `/think`, `!command` | Remove routing/help/autocomplete/docs. `/think` returns removed guidance; `!` returns no-shell guidance and never submits/spawns. |

Generated REPL help, TUI help/autocomplete, agent-message routing documentation, and tests all come from the one authorized registry. `src/channels/repl.rs::print_help`, `crates/thinclaw-channels/src/repl.rs::SLASH_COMMANDS`, and the duplicated `command_catalog.rs` TUI arrays/text are deleted after migration.

## 16. Machine-output and mutation acceptance ledger

| Contract | Exact behavior |
|---|---|
| Record JSON | One `{schema_version:1, command, data}` document on stdout. Empty collections remain `[]` inside `data`. |
| JSONL | One `{schema_version:1, command, type, data}` object per line. Used for natural bounded sequences or live streams. |
| Machine error | Stdout empty; one safe versioned error envelope on stderr; exit 1. Clap owns usage text/exit 2. |
| Unhealthy diagnosis | Complete report envelope on stdout; safe remediation diagnostics on stderr only when human; exit 3. |
| Human | Requested data stdout; warning/progress/deprecation stderr; no banner for immediate/non-TTY commands. |
| Raw artifact | Exact bytes/text only on stdout, or only in atomically published `--out`. No envelope/banner. When written to a file, stdout always contains one status record rendered in the selected output format. |
| Completion | Raw script stdout only regardless of normal presentation defaults. Machine presentation flags that would wrap it are rejected. |
| Color | Machine/artifact/completion never ANSI. `--color always` conflicts with machine output. `NO_COLOR` applies even when empty. |
| Secret | Redact before serialization. Only deliberately requested gateway-token/device-pairing display paths may reveal a generated value under their dedicated guards; passphrases are input-only. A manual experiment auth envelope is a dedicated atomic mode-`0600` file artifact, never stdout/typed data/hash/log, and is single-use/expiring. |
| Confirmation | Machine/non-TTY never prompts. Destructive/high-impact leaves marked `D` require `--yes`; selection-heavy deletion defaults to dry-run. Interactive default focus/answer is cancel/deny. |
| Mutation coherency | Every mutating leaf has one generated `MutationExecutionPolicy`. Finite machine results include request ID, applicable durable/runtime/external IDs/revisions, and exact application state; long-running embedded/process surfaces emit equivalent lifecycle events. Human output states the effect concisely. An active cached-state owner is never bypassed after coordination failure, and stopped-exclusive leaves fail before mutation while any owned/ambiguous runtime is active. |
| Cost | Explicit leaf/help disclosure and bounded requests. Model chat/provider launch probes require their task-specific opt-in; runtime turns/generation remain inherently explicit. |
| Overwrite | Artifact paths refuse existing targets unless a documented `--force`; use private atomic publish and reject symlink/non-regular targets. |

Every finite `M`/`D` leaf above has an exact `case_id` under proof family `MUTATION-policy-routing` in `cli_contract_manifest.json`; `run`/`tui` use exact lifecycle cases under the same family. Each case freezes policy, active/stopped/remote routing, expected receipt/event fields, and whether durable/runtime/external/process IDs apply. The manifest enumerates leaf IDs individually—this paragraph is not wildcard coverage.

## 17. Matrix completion gate

Before deleting this dossier:

1. Generate the canonical tree/reference from actual metadata and diff it against every canonical leaf above.
2. For every table row, link a concrete test name under its proof family. No grouped family may claim coverage without enumerating its leaves.
3. For every supported profile, diff root/category/all help, completion tokens, static capabilities, DB default, service availability, and hidden internal tokens.
4. Compare final tool descriptors to the actual final `ToolRegistry` snapshot by stable identity/origin; no raw-count assertion substitutes for identity parity.
5. Run secret/ANSI/banner/deprecation scans across human, JSON, JSONL, artifacts, errors, boot, TUI buffers, and debug logs.
6. Confirm every `INV-01…INV-95` row, all 124 static tool IDs, all channel sources, all 27 setup step IDs, and every credential-consumer/process-launch descriptor cite an owning task/disposition/proof; update canonical docs, then delete this directory.
