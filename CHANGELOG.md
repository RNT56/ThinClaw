# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** ThinClaw is a fork of [IronClaw](https://github.com/nearai/ironclaw) by NEAR AI.
> Releases prior to v0.13.0 were published under the IronClaw name and are not listed here.
> See the [IronClaw releases](https://github.com/nearai/ironclaw/releases) for earlier history.

## [0.15.0](https://github.com/RNT56/ThinClaw/compare/v0.14.0...v0.15.0) (2026-07-13)


### Features

* **agent:** add agentic-flows routine adapter ([d8e210d](https://github.com/RNT56/ThinClaw/commit/d8e210d56e58fb14e216c93a96bc03692f5ecc96))
* **agent:** enforce disabled_tools deny-list in tool dispatch (T9) ([db027e7](https://github.com/RNT56/ThinClaw/commit/db027e75d0203973c8b95b0f311ab8cc5a673335))
* **agent:** enforce disabled_tools deny-list in tool dispatch (T9) ([51180ea](https://github.com/RNT56/ThinClaw/commit/51180eafe674493040e162767ba21c3a04594717))
* **agent:** harden agentic loop end to end — retry policy, compaction summary reuse, failure backoff ([3ec9086](https://github.com/RNT56/ThinClaw/commit/3ec9086cd2fd55df17080bc96de0f667d38bb2e3))
* **agent:** plan mode (/plan) — propose actions, approve before running ([97d3572](https://github.com/RNT56/ThinClaw/commit/97d3572f3bdffcc3cdfd0b598fddabe4a9fa963f))
* **agent:** tier 2/3 agent-runtime upgrade — concurrency, durability, verified evals ([ecb3cb9](https://github.com/RNT56/ThinClaw/commit/ecb3cb992ea18558412a6c69545726f94b6d2f7d))
* audit-driven agent and loop hardening ([2878418](https://github.com/RNT56/ThinClaw/commit/2878418c6bf0f343fbc1d1d37e9b6f172d578769))
* audit-driven agent hardening, new capabilities, and review fixes ([5550ef1](https://github.com/RNT56/ThinClaw/commit/5550ef189f8ba7ca738687342898a902256784c1))
* **channels:** channel config-schema framework + desktop read commands (TDO-120) ([fa28454](https://github.com/RNT56/ThinClaw/commit/fa28454e8a7d0b169a232dc4af2af7d4811de889))
* **channels:** channel config-schema framework + desktop read commands (TDO-120) ([64b0698](https://github.com/RNT56/ThinClaw/commit/64b06984f373e7414529aced4acdc5ec751a3282))
* **channels:** channel-config submit + Discord schema (TDO-120) ([eaa1fde](https://github.com/RNT56/ThinClaw/commit/eaa1fdeb8d02b1ad4cfdeb627c6dfb870d18d776))
* **channels:** channel-config submit + Discord schema (TDO-120) ([20db2c4](https://github.com/RNT56/ThinClaw/commit/20db2c4f78e50d802a7dcb3526b3a1e49087386d))
* **channels:** extend WIT status-type to cover all StatusUpdate variants (B3) ([6e99b2e](https://github.com/RNT56/ThinClaw/commit/6e99b2e8e6d4f8d3d54d604f9cd3038d05733198))
* **channels:** extend WIT status-type to cover all StatusUpdate variants (B3) ([d5d64af](https://github.com/RNT56/ThinClaw/commit/d5d64afa13e190de379e91a9e779a059b2774f77))
* **cli:** whole-agent encrypted backup — `thinclaw backup` export/import ([d28f5cb](https://github.com/RNT56/ThinClaw/commit/d28f5cbce16168dae0181aae9034949d76fce990))
* **desktop:** advisor + self-repair lifecycle events (TDO-101/102) ([de62319](https://github.com/RNT56/ThinClaw/commit/de62319dc4aefa066bf025ece36c9051dd9d71bc))
* **desktop:** advisor + self-repair lifecycle events (TDO-101/102) ([4bed54f](https://github.com/RNT56/ThinClaw/commit/4bed54f4c4acfc81c46c3640854c1a5d27f35976))
* **desktop:** agent-eval commands — list_envs + run_eval (TDO-113) ([3ad5ad9](https://github.com/RNT56/ThinClaw/commit/3ad5ad991f1348508dda8f87232e8bf82dc39909))
* **desktop:** agent-eval commands — list_envs + run_eval (TDO-113) ([a16b77b](https://github.com/RNT56/ThinClaw/commit/a16b77b40418df2f299d9353ffe22e5d660d4bb9))
* **desktop:** bridge contract foundation + 4 parity issues (compile-verified) ([a57f3af](https://github.com/RNT56/ThinClaw/commit/a57f3af7b66369222184e9c30f58885e352ad8c7))
* **desktop:** Channel Config cockpit panel (TDO-120 UI) ([7ef88bd](https://github.com/RNT56/ThinClaw/commit/7ef88bddfd9c4ba831938d4191c77b9db24eb334))
* **desktop:** Channel Config cockpit panel (TDO-120 UI) ([9379d95](https://github.com/RNT56/ThinClaw/commit/9379d95dcec6cdb93b938fa830b552db4ed7ddc2))
* **desktop:** classify all 346 bridge commands + total-coverage guard (B4) ([357ea92](https://github.com/RNT56/ThinClaw/commit/357ea92edda3a39d6300ae27471c21bfba9e3448))
* **desktop:** classify all 346 bridge commands + total-coverage guard (B4) ([edeb5a0](https://github.com/RNT56/ThinClaw/commit/edeb5a07b45f660b038900fd07c60f11c56347e9))
* **desktop:** cockpit panels — trajectory, rollback, session-search ([4f3ba19](https://github.com/RNT56/ThinClaw/commit/4f3ba19d5e38c53b74a424c48732b9549581fdfa))
* **desktop:** complete bridge linter — classify all gated commands (TDO-002) ([4cb1389](https://github.com/RNT56/ThinClaw/commit/4cb1389f417f5e35da4c8712b0b181d750312e7e))
* **desktop:** complete bridge linter — classify all gated commands (TDO-002) ([e2c2197](https://github.com/RNT56/ThinClaw/commit/e2c21973be06b76a2f57ad1bc68b2c76fdbf6e1d))
* **desktop:** cross-session transcript search command ([2ecc098](https://github.com/RNT56/ThinClaw/commit/2ecc098c87a603035ce1281f5fc565a64addd2bd))
* **desktop:** cross-session transcript search command ([ef8c081](https://github.com/RNT56/ThinClaw/commit/ef8c08196dcb46c0a190a3041cf18a60cee43356))
* **desktop:** finish gating sweep + fix BridgeError serde + regen bindings ([d5c6ad4](https://github.com/RNT56/ThinClaw/commit/d5c6ad44880e170e62bda9b6fcaa0b915fb02b69))
* **desktop:** migrate gated commands to typed BridgeError (B5) ([6724b50](https://github.com/RNT56/ThinClaw/commit/6724b50017ca2a836215326cc3eaf3cb2b3a626a))
* **desktop:** migrate gated commands to typed BridgeError (B5) ([4a822ee](https://github.com/RNT56/ThinClaw/commit/4a822ee2d236e0101910b72e97c085512510b959))
* **desktop:** operator visibility — checkpoints/rollback + trajectory viewer (TDO-103/106) ([19ee5ca](https://github.com/RNT56/ThinClaw/commit/19ee5caa6280d405f18695b15cf7e0005aab89b3))
* **desktop:** overhaul kickoff — bridge contract foundation + first parity slice ([f5d9e74](https://github.com/RNT56/ThinClaw/commit/f5d9e747eff3f1d3dc20395ccaabedeff7eb4b2d))
* **desktop:** rollback/checkpoints cockpit panel ([99c0cee](https://github.com/RNT56/ThinClaw/commit/99c0ceeb051554d4207499c343939cd82a596fad))
* **desktop:** session-search cockpit panel ([92dc3eb](https://github.com/RNT56/ThinClaw/commit/92dc3ebd6538352be97352fa7cd33f5ed46c8800))
* **desktop:** surface context auto-compaction as a lifecycle event (TDO-101/102) ([8ec607c](https://github.com/RNT56/ThinClaw/commit/8ec607c74d4c39b813b6d1a49309586a6c84b92c))
* **desktop:** surface context auto-compaction as a lifecycle event (TDO-101/102) ([dd99bb5](https://github.com/RNT56/ThinClaw/commit/dd99bb5803b4a1e0fb6a4d6051d7a5f0473edd02))
* **desktop:** surface filesystem checkpoints / rollback (TDO-103) ([8f6d4d3](https://github.com/RNT56/ThinClaw/commit/8f6d4d39a1e77f7406bb62e955c26d40323c86c9))
* **desktop:** surface trajectory archive viewer (TDO-106) ([05770a0](https://github.com/RNT56/ThinClaw/commit/05770a076fd9ef929be5458811531095a66c9db2))
* **desktop:** thinclaw_undo / thinclaw_redo commands (TDO-104) ([a0f2873](https://github.com/RNT56/ThinClaw/commit/a0f2873351b20584cdd152cb0d0dc83e11e6b415))
* **desktop:** thinclaw_undo / thinclaw_redo commands (TDO-104) ([8fd7106](https://github.com/RNT56/ThinClaw/commit/8fd71067e5327753283b2c5dc76ac1aba2ab634c))
* **desktop:** trajectory archive cockpit panel ([5c7817d](https://github.com/RNT56/ThinClaw/commit/5c7817d54022b556aaa1ce990431df3bb95ec10b))
* **desktop:** undo/redo buttons in cockpit chat toolbar (TDO-104 UI) ([e4d0186](https://github.com/RNT56/ThinClaw/commit/e4d0186ec40990fd76dca34d09d99bec28838ffb))
* **desktop:** undo/redo buttons in the cockpit chat toolbar (TDO-104 UI) ([10056b8](https://github.com/RNT56/ThinClaw/commit/10056b8b387d3b6ffc147344813b16f558204dde))
* **gateway:** B1 device identity — QR pairing, scoped tokens, TLS listener, push-pull approvals ([1d41742](https://github.com/RNT56/ThinClaw/commit/1d41742ec250e0e4599a7dd43d2795756db09a1c))
* **gateway:** B2 first-party push — ApnsPusher, device push registration, content-free notifier ([7d6b00e](https://github.com/RNT56/ThinClaw/commit/7d6b00e4d3913ef4ce33b9ec0ccb5e571947fb86))
* **gateway:** opt-in RBAC principals with role-scoped capability gating ([40b16e0](https://github.com/RNT56/ThinClaw/commit/40b16e085dd30d6141b8cd52d3a7ac8a82e12ae4))
* merge audit hardening stack to main ([bda7a61](https://github.com/RNT56/ThinClaw/commit/bda7a61f492040b275fae2fb7f49509c381e2fcd))
* **mobile:** B3 LAN discovery + M2 iOS approvals & push ([886901b](https://github.com/RNT56/ThinClaw/commit/886901bcdfd846f83786aad17838d151dfc8654a))
* **mobile:** iOS surface foundation — docs, gateway OpenAPI contract, apps/ios scaffold ([5afe64e](https://github.com/RNT56/ThinClaw/commit/5afe64e47993fc19a78dec48795e6d2be2ea68ce))
* **mobile:** M1 iOS pairing + streaming chat — onboarding, transport, GRDB, app graph ([4749e13](https://github.com/RNT56/ThinClaw/commit/4749e132a8fd8ee02851c7119e71b4aed137d249))
* **mobile:** M3 widgets + Live Activity — run-tracking, snapshot pipeline, 4 widgets ([6bfa254](https://github.com/RNT56/ThinClaw/commit/6bfa254c59db682179b712db7653d1842d324516))
* **mobile:** M4 watchOS companion — companion tokens, relay, wrist approvals ([373251e](https://github.com/RNT56/ThinClaw/commit/373251e2775e26e8a98270dec92568b2740d4506))
* **mobile:** M5 polish + TestFlight prep — settings, jobs, accessibility, release pipeline ([0c886e8](https://github.com/RNT56/ThinClaw/commit/0c886e8b6723803989f09a6cbb9e14b4642bc1b5))
* **observability:** add rolling daily file log sink (A3) ([048a670](https://github.com/RNT56/ThinClaw/commit/048a670f33b260e4bcfca369dc66862f428aa8b0))
* **observability:** emit 5 dead ObserverEvent variants + default observer to log (B2) ([c7f53af](https://github.com/RNT56/ThinClaw/commit/c7f53af180995d57b8fee186e408b7b2f4139cac))
* **observability:** emit the 5 dead ObserverEvent variants + default observer to log (B2) ([f396159](https://github.com/RNT56/ThinClaw/commit/f39615903d61f7d89ff704043ff4e962cdde5215))
* **observability:** real /api/health readiness probe (A4) ([942f5d6](https://github.com/RNT56/ThinClaw/commit/942f5d69c40596d1babf85b35cc2425b5c4eea69))
* **observability:** real /api/health readiness probe [A4] ([625fac2](https://github.com/RNT56/ThinClaw/commit/625fac23f4e136f8f5d51de5adb4b48e015784d2))
* **observability:** rolling daily file log sink [A3] ([ba9e3f5](https://github.com/RNT56/ThinClaw/commit/ba9e3f5125a3870fbbc12ae995374ab54c2ba475))
* **remediation:** execute F-01..F-19 follow-ups (trust boundaries, vision wiring, platform polish) ([85ca082](https://github.com/RNT56/ThinClaw/commit/85ca082c645776b3e2a8889f02de2f331025c1ae))
* **repo-projects:** Repository Project Supervisor — live GitHub pipeline + connector ([6814056](https://github.com/RNT56/ThinClaw/commit/681405665da03d5d446fafe62ca7a373a42a7b41))
* **repo-projects:** Repository Project Supervisor — live GitHub pipeline + connector ([723dab7](https://github.com/RNT56/ThinClaw/commit/723dab70c4d74b09a330025a16d2fa8b90e7e1c4))
* **tools:** argument-scoped tool permission rules ([2b4b048](https://github.com/RNT56/ThinClaw/commit/2b4b048f6c95999356a19b63991eea24c7da1811))
* **tools:** opt-in host-direct filesystem confinement (seatbelt/bwrap) ([004576b](https://github.com/RNT56/ThinClaw/commit/004576b01912b89b9eb9abb9e2ca75c7c79e0b6b))
* **voice:** F-18 — route wake-word utterances into the agent dispatcher ([28f76f7](https://github.com/RNT56/ThinClaw/commit/28f76f7f249e858e59c25e2b6b45896b1b20e3fd))


### Bug Fixes

* **agent:** address max-effort review findings — parser panic, fail-closed inversion, cache sharing ([dbbe587](https://github.com/RNT56/ThinClaw/commit/dbbe58733c65cc9ef3fab7e5d71db6a6b1b22fd4))
* **agent:** second-pass review findings — cache isolation, ledger first-write-wins, worker panics surfaced ([019f46e](https://github.com/RNT56/ThinClaw/commit/019f46e755ce61e7b6fb84e81c9f8f994c7e232a))
* **async:** remove blocking syscalls from async fns (P0) ([6870e11](https://github.com/RNT56/ThinClaw/commit/6870e11b6dc5338cfa6490b75988827a722b19a7))
* **async:** remove blocking syscalls from async fns (P0) ([307638a](https://github.com/RNT56/ThinClaw/commit/307638aa945732e332ce850a80bab8a4275ed20f))
* **channels:** commit Cargo.lock for the channels-core serde dep (TDO-120) ([fb9dea6](https://github.com/RNT56/ThinClaw/commit/fb9dea653d0f94cb6dbd277008545c58201f7757))
* **ci:** clippy redundant-closure + quick-xml advisory risk-acceptance ([abc789f](https://github.com/RNT56/ThinClaw/commit/abc789f1e459b5658775d67df98625fb0315f58f))
* **ci:** close the five CI failures on the merged branch ([91c767f](https://github.com/RNT56/ThinClaw/commit/91c767fc6f4f857508c7d76eb696828aeb99359d))
* **ci:** green the reduced-feature profiles + Desktop Companion disk space ([539fadd](https://github.com/RNT56/ThinClaw/commit/539faddaffc9c8d77cae86e376697bddb5b53ac6))
* **ci:** resolve the 13 red CI jobs (clippy dead-code, stale test, voice/full ALSA) ([e7df353](https://github.com/RNT56/ThinClaw/commit/e7df353fa9c57f9e0d1b1ca0eefdd6ce4e277a3e))
* cross-platform TLS key writer, stale desktop lockfile, iOS CI toolchain ([f0b1655](https://github.com/RNT56/ThinClaw/commit/f0b1655cb86b20b9db2be8c724edf2757798353e))
* **deps:** remediate desktop advisories + add desktop advisory CI scan (P0) ([c030f15](https://github.com/RNT56/ThinClaw/commit/c030f157c5574123f7f89d813a9c02f30a8e9a4a))
* **deps:** remediate desktop advisories + add desktop advisory CI scan (P0) ([7765917](https://github.com/RNT56/ThinClaw/commit/776591772b3aa4b6282282f38f8e1b4b71f61120))
* **desktop:** avoid get_current_pid() panic in system specs [A11 desktop] ([f7169a0](https://github.com/RNT56/ThinClaw/commit/f7169a05155df17735c0cdc872b035ec197a5e6e))
* **desktop:** bump @vitejs/plugin-react to 6.x for vite 8 ([dde1683](https://github.com/RNT56/ThinClaw/commit/dde1683c0eabfed6c37c8fbc56e6352ec04fc2b6))
* **desktop:** classify thinclaw_session_search in ROUTE_TABLE (bridge linter) ([55f7722](https://github.com/RNT56/ThinClaw/commit/55f7722bfea8c2eb2e045b7eae985c4e76dd57d6))
* **desktop:** correct channel stream-mode key + vocabulary (T10) ([38bbec6](https://github.com/RNT56/ThinClaw/commit/38bbec6932a2f0610f12f417b9571da99ac2504c))
* **desktop:** correct dishonest UI states and an empty-row chat regression ([#101](https://github.com/RNT56/ThinClaw/issues/101)) ([ee5c3b1](https://github.com/RNT56/ThinClaw/commit/ee5c3b1e57274dc4aa9c84c0b8ddc585e90d48a3))
* **desktop:** de-flake sub_agent_registry tests (global-registry parallel race) ([#112](https://github.com/RNT56/ThinClaw/issues/112)) ([726ecda](https://github.com/RNT56/ThinClaw/commit/726ecda8e64c3f0c4f2065dabc4ba4db66816a9c))
* **desktop:** don't panic on get_current_pid() in system specs (A11) ([043219d](https://github.com/RNT56/ThinClaw/commit/043219d495d51a7bf26ea5b33f40c07f1ee83d1a))
* **desktop:** honest sidecar status — image/tts 'configured' not 'running' (T8) ([182fec5](https://github.com/RNT56/ThinClaw/commit/182fec5cadfb0b0a02deaedb4190f0f48ba41cc7))
* **desktop:** honest sidecar status — image/tts 'configured' not 'running' (T8) ([22290d9](https://github.com/RNT56/ThinClaw/commit/22290d93f7514feed61f8a29e64969bc0eaa0dc4))
* **desktop:** honest skill toggle, dashboard stats & version (T5/T7) ([8d13ecd](https://github.com/RNT56/ThinClaw/commit/8d13ecd9e378ebc4e03fd05f50978244734715ea))
* **desktop:** make skill toggle, dashboard stats & version honest (T5/T7) ([2e97992](https://github.com/RNT56/ThinClaw/commit/2e979926f8b1325108dcc95cd6b4c87d1fbc8502))
* **desktop:** persist Gmail label filter; remove dead WhatsApp QR login ([#106](https://github.com/RNT56/ThinClaw/issues/106)) ([1a76a47](https://github.com/RNT56/ThinClaw/commit/1a76a478d00f44d5777d32894f0a16908841c167))
* **desktop:** play cloud TTS (MP3) correctly in Read Aloud ([ed99b64](https://github.com/RNT56/ThinClaw/commit/ed99b64410ee6c9c29aeb444f8de40337a63d8c4))
* **desktop:** play cloud TTS (MP3) correctly in Read Aloud ([fd3936f](https://github.com/RNT56/ThinClaw/commit/fd3936ffbb8765d186d2a59e316ce8b9aaa99a7f))
* **desktop:** refresh backend Cargo.lock for the B3 mdns/base64/sha2 deps ([a6f6392](https://github.com/RNT56/ThinClaw/commit/a6f6392faf435991d3e35464e35456b1b09a7cac))
* **desktop:** replace removed lucide 'Github' brand icon with FolderGit2 ([9a7c599](https://github.com/RNT56/ThinClaw/commit/9a7c59911a088d3e3249c384eaca802fb79a0ae0))
* **desktop:** write the correct channel stream-mode key + vocabulary (T10) ([e040405](https://github.com/RNT56/ThinClaw/commit/e04040502f884cec6aa267ee95a5b4853a9d82bf))
* **ios:** add companion op stubs to ThinClawTransport's MockGatewayClient ([bb69129](https://github.com/RNT56/ThinClaw/commit/bb6912927e96955364eb4bd772a561ac65876083))
* **ios:** serialize SSE watchdog activity updates ([#236](https://github.com/RNT56/ThinClaw/issues/236)) ([09cd242](https://github.com/RNT56/ThinClaw/commit/09cd2422b64dbc595a47d7af50f8c6d3808314dd))
* **learning:** invalidate ready-provider cache on bulk settings import ([bd1c752](https://github.com/RNT56/ThinClaw/commit/bd1c75242fd0f877c217ffec264be47fec5b3d62))
* **mobile:** iOS follow-ups — assistant_thread schema, live watch relay, CI tuist/feature-test lanes ([1040935](https://github.com/RNT56/ThinClaw/commit/1040935f565af75ce99f67a669fc2fe680dec894))
* **panics:** OnceLock hot-path regexes/selectors + recoverable vision error (P1) ([bf1d9d4](https://github.com/RNT56/ThinClaw/commit/bf1d9d46aca1866c1f10daf953d3caa12fc7708c))
* **panics:** OnceLock hot-path regexes/selectors + recoverable vision error (P1) ([354a1d8](https://github.com/RNT56/ThinClaw/commit/354a1d861d36ddc875488283c50c17dfb345e429))
* **panics:** replace hot-path expect/parse panics with typed errors (A11) ([4a053e0](https://github.com/RNT56/ThinClaw/commit/4a053e08830ee7628377a8e6f1cdac012ec228bb))
* **panics:** typed errors for hot-path expect/parse panics [A11 root subset] ([177578f](https://github.com/RNT56/ThinClaw/commit/177578f936abb0e910303768e837da9159f90d95))
* **security:** eliminate RUSTSEC-2026-0187 at the source (pdf-extract 0.7→0.12, lopdf 0.42) ([16b3077](https://github.com/RNT56/ThinClaw/commit/16b3077a9923593e6aa39672aa450f9d59c77cc4))
* **security:** harden injection regex, audit in-memory secret access, warn on token-in-URL (A10) ([7965519](https://github.com/RNT56/ThinClaw/commit/7965519d7409b14f447cd1352c1f2d51ec25efd1))
* **security:** injection regex + in-memory secret audit + token-in-URL warning [A10 subset] ([612239a](https://github.com/RNT56/ThinClaw/commit/612239a51a00e7001322dfe83ae0bc0e223799ae))
* **security:** patch RUSTSEC-2026-0188 (wasmtime-wasi) + RUSTSEC-2026-0190 (anyhow) ([767f640](https://github.com/RNT56/ThinClaw/commit/767f640b4549e41aeccd5dc1d3cdae9589350905))
* **security:** patch RUSTSEC-2026-0188 (wasmtime-wasi) + RUSTSEC-2026-0190 (anyhow) ([973c45e](https://github.com/RNT56/ThinClaw/commit/973c45e8a689fb17ae56e8c92811d2c543c00b47))
* **security:** remove orphaned DangerousToolTracker + fix trusted-proxy CIDR (P0) ([baa86e9](https://github.com/RNT56/ThinClaw/commit/baa86e9f14770bd7128bd60eef83386ccd28e223))
* **security:** remove orphaned DangerousToolTracker + fix trusted-proxy CIDR (P0) ([db1e218](https://github.com/RNT56/ThinClaw/commit/db1e218474614b8ff2f33c426e0e886b27baa26c))
* **security:** resolve RBAC route-coverage gap + pre-auth KDF/zip DoS ([8abba43](https://github.com/RNT56/ThinClaw/commit/8abba437a0f5ed6009b62cf2658b8b3ace755272))


### Performance Improvements

* **skills:** load-then-swap skill reload (off-lock discovery) [A9] ([92659f6](https://github.com/RNT56/ThinClaw/commit/92659f61552aace949acae5e815fdef005b13eaa))
* **skills:** load-then-swap skill reload to avoid write-lock-held discovery IO (A9) ([3c4bd22](https://github.com/RNT56/ThinClaw/commit/3c4bd22b4477af3c1d115a284fc10abeac3dffa4))

## [Unreleased]

### Added

- Native Swift iOS + watchOS surface (`apps/ios`): device pairing (QR + scoped tokens, TLS listener), streaming chat, push-pull approvals, LAN discovery, APNs push, home-screen widgets with Live Activity run tracking, and a watchOS companion with companion tokens/relay and wrist approvals.
- Typed Rust client SDK (`crates/thinclaw-client`) wrapping the gateway HTTP+SSE surface (send/stream/history/threads/approvals) with an OpenAI-compatible fast path and tokens redacted from `Debug`.
- Prometheus observability backend with `GET /metrics`.
- Real `GET /api/health` readiness probe: 200 only when the database is reachable, an LLM provider is configured, and an inbound channel is wired; otherwise 503.
- Plan mode (`/plan`): propose actions and approve before running.
- Unified `/rewind` command: restores both conversation (to turn N) and files via turn-tagged shadow-git checkpoints, with a non-destructive list/dry-run.
- Argument-scoped tool permission rules.
- Zero-config `web_search` built-in tool (keyless DuckDuckGo, SSRF-guarded, size-capped).
- MCP health monitor that populates `McpRuntimeHealth` and auto-reconnects crashed stdio servers with per-server exponential backoff.
- Loop-hardening observability: 8 new `thinclaw_loop_*` metric series covering loop and phase starts, stops, iterations, retries, and timing.
- `ExtensionKind::NativePlugin`: signature-gated native plugin loading, default-off.
- Discord Ed25519 webhook signature verification (host-side).
- Repo-project supervisor: GitHub App-backed automation with a live pipeline and connector, plus an autoplanner behind the `REPO_PROJECTS_AUTOPLAN` env gate (default-off).
- Opt-in gateway RBAC with role-scoped capability gating.
- Opt-in host-direct filesystem confinement (seatbelt/bwrap).
- First-party push: `ApnsPusher`, device push registration, and a content-free notifier.
- Whole-agent encrypted backup: `thinclaw backup` export/import.
- Desktop cockpit panels for session search, trajectory archive, rollback/checkpoints, and channel config, plus an undo/redo chat toolbar and advisor/self-repair/context-compaction lifecycle events.
- Rolling daily file log sink.

### Changed

- Landed a 13-workstream audit-driven remediation across security, DB correctness, WASM channels, self-repair, desktop, experiments, LLM routing, docs, and test/CI infrastructure.
- Hardened native and WASM channel lifecycles with owned shutdown/drain paths, bounded reconnect behavior, APNs HTTP/2 support, Gmail token refresh, and channel-manager ownership of hot-reload stream forwarders.
- Decomposed 10 historical god-files into focused directory modules, and added a CI god-file size guard (`MAX_LINES=2000`).
- Consolidated history/store onto `thinclaw-db`.
- Completed the `thinclaw-media` crate migration, slimming `src/media` to a facade.
- Marked `StatusUpdate` `#[non_exhaustive]` with fallback match arms.
- `ROUTE_TABLE` now classifies all 346 desktop bridge commands, enforced by a total-coverage CI test.
- Migrated gated desktop commands to a typed `BridgeError`.
- Hardened the agentic loop end to end: retry policy, compaction summary reuse, and failure backoff.
- Gated sub-agents by the shared `CostGuard` with an optional per-principal concurrency cap (`SUBAGENT_MAX_PER_PRINCIPAL`).
- Migrated the desktop frontend to Tailwind CSS v4, consolidated npm and MSRV-compatible Rust major upgrades, and added a `[workspace.dependencies]` table for shared deps.
- Marked ThinClaw Desktop as experimental.
- Refreshed channel, APNs/browser-push, ComfyUI media-generation, setup, CLI, tool/skill, parity, and dependency documentation to match the current native lifecycle, built-in sidecar, and WASM channel architecture, while keeping real-provider smoke requirements clearly marked as credential-gated validation.
- Removed stale audit-planning closure documents from the current docs tree.

### Fixed

- Closed the empty `gateway_auth_token` auth bypass at both layers by filtering empty/whitespace tokens.
- OAuth state is now generated and constant-time-validated end to end (CSRF).
- Added DNS-rebinding protection via host pinning and fixed the trusted-proxy CIDR check.
- Sanitized libSQL FTS5 `MATCH` injection.
- Confined sandbox proxy credentials behind a store-backed resolver.
- Enforced WASM table/instance resource limits.
- Fixed the multibyte-UTF-8 `split_message` panic in the telegram, slack, and discord channels.
- Added routine run-lease zombie reaping.
- Fixed streaming `finish_reason` for tool calls.
- Serialized iOS SSE activity updates so comment keep-alives reset the watchdog before timeout evaluation, eliminating a reconnect race under load.
- Rebuilt the runtime-contract OpenAPI export around one shared component registry so every generated `$ref` resolves inside the document.
- Guarded the `image_gen` progress divide-by-zero.
- Replaced hot-path `expect`/parse panics with typed errors and moved `OnceLock` hot-path regexes/selectors off the hot path.
- Removed blocking syscalls from async functions.
- Added Gmail unattended OAuth token refresh (proactive + on-401) so long-running deployments don't silently stop.
- Restored cross-turn tool-result continuity: `Thread::messages()` reconstructs prior turns' tool calls and results.
- Invalidated the learning ready-provider cache on bulk settings import.
- Resolved an RBAC route-coverage gap and a pre-auth KDF/zip DoS.

### Removed

- Erased roughly 7K lines of verified-dead code, including 14 `src/safety` orphans, 3 unwired CLI modules, the `self_message` anti-loop module, the `qr_pairing` scaffold, the tailscale identity module, the standalone heartbeat runner, the `SmartRoutingProvider` decorator, and the `InferenceRouter` chat modality.
- Removed the orphaned `DangerousToolTracker`.
- Removed dead desktop frontend code, dead web-search probe commands, and the dead WhatsApp QR login.

### Security

- Constant-time OAuth-state comparison via `subtle::ConstantTimeEq`.
- `subtle`-based constant-time comparisons in `thinclaw-tools` for pairing secrets, webhook secrets, and device tokens.
- Added desktop backend advisory scanning in CI (`cargo deny check advisories`) with a desktop-scoped `deny.toml`.
- Zero cargo-deny advisory ignores (`deny.toml` `[advisories] ignore = []`).
- Patched RUSTSEC-2026-0187 (pdf-extract/lopdf), RUSTSEC-2026-0188 (wasmtime-wasi), and RUSTSEC-2026-0190 (anyhow) at the source.
- Disabled JSON Schema's external HTTP and file resolvers for untrusted tool schemas; external references now fail closed.
- Hardened the injection regex, audited in-memory secret access, and warn on token-in-URL.

## [0.14.0](https://github.com/RNT56/ThinClaw/releases/tag/v0.14.0) - 2026-05-14

### Added

- ComfyUI-backed media generation with native `image_generate`, `comfy_health`, `comfy_check_deps`, `comfy_run_workflow`, and approval-gated `comfy_manage` tools.
- ComfyUI REST/WebSocket workflow execution, API-format workflow validation, output sanitization, dependency scanning, and bundled starter workflows.
- Trusted `creative-comfyui` skill, ComfyUI configuration/settings, CLI commands, and documentation for local/cloud setup and generation.
- Renderable generated-media artifacts in web gateway tool results.

## [0.13.7](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.7) - 2026-04-24

### Added

- ACP v1 compatibility work for editor clients, including typed wire messages, JSON-RPC transcript coverage, prompt/session lifecycle handling, permission round-tripping, client filesystem/terminal bridges, MCP stdio descriptor wiring, and stdout cleanliness checks.
- AgentEnv and research campaign plumbing with benchmark adapters, Research WebUI/API surfaces, and trajectory metadata for token/logprob capture when providers support it.
- Extension manifest foundations for tool, channel, memory, context, and native-plugin contributions, with native plugin loading gated behind explicit unsafe configuration and signature metadata.
- WASM tool host-mediated invocation support through declared aliases with policy, approval, timeout, recursion-depth, and audit controls.
- Provider-native streaming capability metadata and streaming paths across the LLM stack, with simulated streaming retained only as an explicit fallback.
- Release and deployment improvements for Linux, Docker Compose, gateway access, readiness probes, WASM extension bundles, and build-profile documentation.

### Changed

- ACP capability advertisement now tracks implemented and tested behavior instead of exposing placeholder features.
- Release workflow validation now fails when WASM extension manifests are missing sources, capabilities, or bundle outputs instead of publishing incomplete artifacts.
- Setup wizard tests and docs now agree on the documented 12-step quick setup flow.
- User-tools documentation now prefers the canonical `~/.thinclaw/user-tools/` path while preserving the legacy underscore path as an alias.

### Fixed

- Docker availability detection no longer hangs when Docker Desktop or compatible runtimes leave `docker version`/`docker info` blocked; CLI probes now have killable per-command timeouts.
- Dispatcher streaming tests now model native streaming support explicitly, matching the production gate that avoids fake progressive streaming for non-native providers.
- ACP local validation, default build checks, feature-profile checks, and the broad library suite are green with the Docker hang fixed.
- Trusted-proxy web gateway identities no longer accept compatibility user/actor override parameters intended only for bearer-token development paths.
- Cargo-deny advisory coverage is clean after updating `rustls-webpki`, and default/full clippy gates are clean under `-D warnings`.
- Web gateway CORS now uses the actual bound listener port for ephemeral port binds.

## [0.13.6](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.6) - 2026-04-14

### Added

#### Routing Engine V2
- Unified `RoutePlanner` with three strategies: `Solo`, `Failover`, and new `AdvisorExecutor` mode (executor lane runs the turn, advisor lane consults and auto-escalates on risky or complex turns)
- Dispatcher interception layer: all outbound LLM calls flow through the routing policy
- Live cutover from legacy routing — no migration required
- Routing telemetry: per-call latency, token counts, and strategy-hit histograms
- Health signal tracking for automatic failover provider selection

#### Self-Improving Learning Runtime
- Closed-loop learning system: the agent reviews its own conversations, extracts patterns, and refines future behaviour
- Conversation recall store for pattern matching across sessions
- Learning conversation analysis pipeline with configurable feedback thresholds

#### Agent Subsystems
- **Checkpoint system**: durable mid-turn state snapshots for crash recovery and long-running tasks
- **Personality overlay engine**: adaptive tone/style selection based on conversation context and channel type
- **Session search**: full-text search across all agent sessions with relevance ranking
- **Context monitor**: proactive context-window utilisation tracking with compaction triggers

#### Server Decomposition (Monolith Breakup)
- Extracted 13 dedicated handler modules from the 7,900-line `server.rs` monolith: `chat`, `experiments`, `extensions`, `gateway`, `jobs`, `learning`, `logs`, `memory`, `pairing`, `projects`, `providers`, `routines`, `settings`
- New shared modules: `identity_helpers`, `rate_limiter`, `static_files`
- Each handler module is independently testable with clear API boundaries

#### New Built-in Tools
- `browser_a11y`: accessibility-tree-based page interaction for headless browser automation
- `browser_cloud`: cloud browser service integration (Browserbase, Steel, etc.)
- `execute_code`: sandboxed code execution in multiple languages
- `search_files`: recursive file search with glob patterns and content matching
- `clarify`: structured clarification requests to disambiguate user intent
- `send_message`: cross-channel message delivery from within tool pipelines
- `skill_tools`: runtime skill management (install, reload, inspect)
- `learning_tools`: manual learning annotation and pattern review
- `moa` (Mixture of Agents): fan-out queries across multiple models and merge results
- `advisor`: strategic consultation tool for the AdvisorExecutor routing strategy
- Enhanced `browser` tool with screenshot capture, element interaction, and cookie management
- Enhanced `shell` tool with working directory tracking and environment variable passthrough

#### Safety and Security
- **PII redactor**: regex + NER-based PII detection and masking for logs and stored conversations
- **Smart approve**: risk-scoring engine for tool calls with configurable thresholds
- **OSV vulnerability checker**: proactive dependency scanning against the OSV database
- Enhanced `sanitizer` with HTML/XSS stripping and prompt-injection pattern detection

#### Skill System Expansion
- `github_source`: install skills directly from GitHub repositories (public and private)
- `remote_source`: fetch skills from arbitrary HTTPS URLs with SHA256 verification
- `well_known_source`: curated skill catalog discovery via `.well-known/thinclaw-skills.json`
- `quarantine`: sandboxed skill staging with integrity checks before promotion to active use
- Skill file watcher for automatic hot-reload on disk changes

#### Personality Pack Library
- 6 production-ready personality packs: `default`, `creative_partner`, `mentor`, `minimal`, `professional`, `research_assistant`
- Psychographic identity profiles with tonal overlays and channel-aware formatting hints

#### TUI Skin System
- TOML-based terminal skin engine with runtime switching
- 6 bundled skins: `athena`, `cockpit`, `delphi`, `midnight`, `olympus`, `solar`
- Per-skin colour palettes, glyph sets, and layout tuning

#### Database and Migrations
- 8 new migrations (V10–V17): identity registry, actor-scoped history, job context, agent capability isolation, learning tables, experiments platform, research tables, experiment cost breakdown
- Schema divergence detection test suite with allowlist-based governance
- Database contract tests enforcing libSQL/Postgres feature parity
- Enhanced Postgres backend with full learning, experiment, and identity store support

#### CI and Build
- Dedicated `light` profile test job: validates the default user-facing build separately from the full-feature build
- Consolidated CI: merged `code_style.yml` and `test.yml` into a single `ci.yml` workflow
- Codecov integration for coverage tracking
- `build.rs` for compile-time asset embedding and version metadata

#### WebUI Overhaul
- Routing strategy configuration: mode selector tiles (Solo / Failover / AdvisorExecutor) with conditional advisor config fields
- Provider management redesign: slot-based editor with credential sync, connection testing, and model discovery
- Cost dashboard: daily vertical bar chart, time-range selector (7d/30d/90d), budget progress bars
- Experiments UI: create, monitor, and compare A/B experiments with cost breakdown
- Learning insights panel: review extracted patterns and agent self-improvement metrics
- Modern toggle switches replacing all binary checkboxes
- Responsive layout overhaul with mobile-first grid system

#### LLM Infrastructure
- `credential_sync`: encrypted at-rest credential storage with secure keychain integration
- `model_guidance`: per-model capability hints (supports vision, supports tools, max context, etc.)
- `runtime_manager`: centralised LLM lifecycle management with connection pooling
- `usage_tracking`: per-session and per-agent token and cost accounting
- Enhanced `reasoning` module with chain-of-thought extraction and structured output parsing
- Enhanced `provider_factory` with lazy initialisation and provider health probing

### Fixed

- Learning conversation recall store path was missing from the database layer
- Subagent executor now correctly propagates cancellation signals to child tasks
- WebUI settings page uses full browser width (removed legacy `max-width` constraint)
- Telegram HTML formatter handles nested bold/italic/code spans correctly
- WASM channel wrapper correctly forwards capability JSON updates
- Snapshot tests updated to reflect new settings schema
- Provider editor credential fields preserve values across tab switches

### Changed

- Bumped Rust edition to 2024, minimum toolchain to 1.92
- Cargo.lock updated with 50+ dependency upgrades
- Removed stale planning documents: `rewrite-docs/`, `docs-audit/`, `Agent_flow.md`, `audit.md`, `database_divergence_plan.md`
- Comprehensive README rewrite with updated architecture diagrams and feature matrix
- CLAUDE.md updated with current module map, test patterns, and contribution guidelines
- FEATURE_PARITY.md refreshed against current codebase capabilities
- Expanded `.gitignore` with OS files, local databases, IDE settings, log patterns, and build artifacts

## [0.13.2](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.2) - 2026-03-28

### Added

- Multimodal media pipeline for all channels — images, audio, video, and documents are downloaded and routed to the LLM across Telegram, Discord, Signal, iMessage, WhatsApp, and Slack
- Discord native channel: CDN attachment download with 20MB size limit
- Signal channel: typed `SignalAttachment` struct, reads binary from signal-cli's local attachment store
- iMessage channel: queries `attachment` + `message_attachment_join` tables from chat.db, reads files from disk
- WhatsApp WASM channel: 2-step Cloud API media download (media URL → binary), supports image/audio/video/document/sticker with captions
- Slack WASM channel: file download via `url_private_download` with Bearer auth
- WIT `media-attachment` record and `attachments` field on `emitted-message` for WASM channel binary media transport
- WASM host boundary: 20MB per-file and 50MB per-message attachment size limits
- BOOT.md startup hook: pre-reads workspace docs instead of relying on tool calls
- Multi-provider LLM routing: Provider Vault, agent model switching, wizard fallback step
- Runtime-configurable Claude Code model/max-turns via WebUI
- Active channel names injected into LLM system prompt
- Settings page UX overhaul: subtabs, collapsible sections, search
- Provider Vault moved to dedicated tab with enhanced broadcast logging
- Broadcast support for WASM channels via `on_respond`
- BOOT.md startup briefing: daily logs, memory, heartbeat greeting with DB persistence

### Fixed

- Apple Mail timestamps showing year 2057 (+31 year offset)
- Telegram polling timeout mismatch causing 409 conflicts
- Telegram webhooks unreachable — tunnel forwards to wrong port
- Telegram falls back to polling when webhook fails + Provider UX fixes
- Numeric-looking strings (chat IDs) in `Option<String>` settings
- Detect and bail on tailscale funnel startup failures
- Bail on empty Tailscale hostname instead of producing broken URL
- Telegram broadcast delivery + thread deletion in WebUI
- Boot hook delivery: Telegram, WebUI persistence, BOOT.md migration
- `memory_tree` fails with 'Input cannot be empty' on default params
- Settings array parsing, model reset, provider auto-enable, XSS
- Apple Mail polling crash + add apple_mail search/send tool
- Apple Mail `allow_from` wired to DB settings + security warning
- Prevent BOOTSTRAP.md from re-executing on every restart

### Other

- Update WASM artifact URLs and SHA256 checksums

## [0.13.0](https://github.com/RNT56/ThinClaw/releases/tag/v0.13.0) - 2026-03-26

### Added

- Apple Mail channel + auto-start for macOS apps
- Notion WASM tool with full API coverage
- WebUI model routing settings + wizard cross-provider API key collection
- `/restart` command, hardened auto-approve, improved deployment docs
- DB-backed config resolvers, wizard local tools/sandbox clarity, bootstrap identity injection
- WebUI Settings tab, agent bootstrap/cleanup fixes, favicon update
- MCP stdio transport for MCP servers
- Dual WASM extension deployment options (download from releases vs bundled in binary)
- Wasmtime 36 upgrade, sysinfo API migration, module restructuring
- Full IronClaw agent engine integration + codebase audit

### Fixed

- `super::` prefix for test references to private constants
- Removed placeholder OAuth secrets, correctly documented auth models
- Corrected registry URLs and CI manifest lookup for slack-tool and telegram-mtproto
- Complete IronClaw → ThinClaw rebrand + fixed failing streaming test
- Stripped NEAR AI author references, fixed Linux build errors
- Updated cargo-dist version to 0.31.0 in release.yml
- Corrected WiX manufacturer whitespace to match cargo-dist expectation
- Updated WiX installer manifest from ironclaw to thinclaw
- Installed libasound2-dev for voice feature in CI
- Patched artifact download URLs into manifests, updated repo URLs
- Clear error when cargo-component missing, soft-fail bundled channels
- Wizard: 3 bugs — missing persist, silent fallback skip, loop allocations
- Wizard: bundled-wasm extraction for tool install step (top-priority install path)
- Registry: removed placeholder artifacts, allow sha256-null fallback
- Deployment bugs — desktop feature, repl guards, registry artifacts
- Rebranded Telegram/Discord channels and docs to ThinClaw

### Other

- Added `channels-docs/` with documentation for all 12 channels
- Added `tools-docs/` with documentation for all 11 WASM tools
- Added Gmail setup guide and fixed stale libSQL limitation claims
- Updated FEATURE_PARITY, CLAUDE.md, and setup README
- Added consolidated External Dependencies guide
- Added data directory layout, upgrade workflow, and reset procedures
- Docker worker & Claude Code infrastructure
- Applied cargo fmt to entire codebase
- Updated WASM artifact URLs and SHA256 checksums
- Removed dead code, wired full search pipeline into libsql
- Comprehensive CLAUDE.md update against actual codebase
- Renamed all IRONCLAW_* env vars to THINCLAW_*
- Rebranded all extension sources from IronClaw to ThinClaw
- Added comprehensive deployment guide
