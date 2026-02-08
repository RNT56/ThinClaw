Security review checklist for your desktop companion app + OpenClaw integration

This is written as a practical “ship/no-ship” checklist you can use for PR reviews, releases, and audits.

⸻

1) Threat model & scope

1.1 Define supported deployment topologies
	•	Document which modes are officially supported (Local Gateway, Remote Gateway).
	•	Document what “Node Host ON” means in each mode (what machine is controllable).
	•	Document whether local inference may be exposed to remote gateways and how.

1.2 Identify trust boundaries (must be explicit in docs + UI)
	•	Gateway host is a trust domain (runs agent loop + tool policy).
	•	Node host machine is a trust domain (executes OS-level tools).
	•	Client UI machine is a trust domain (holds user credentials, may host inference).
	•	Model providers are external trust domains (cloud inference, logging policies).

1.3 Enumerate attacker types
	•	Malicious prompt injection (web pages, emails, PDFs, chat messages).
	•	Rogue remote gateway operator (if user connects to a gateway they don’t own).
	•	Local malware on user machine reading your app’s files/keys.
	•	Man-in-the-middle (WS, HTTP inference endpoints).
	•	Supply chain attacks (sidecar, auto-update, install script).

⸻

2) “Safe by default” behavior

2.1 Default install posture
	•	Profile A (safe) does not enable node host or request OS permissions.
	•	Remote Gateway mode defaults to no local node host and no inference exposure.
	•	Local inference defaults to localhost binding only.

2.2 Runtime toggles enforce safety
	•	Switching to Remote Gateway stops any local node host unless user explicitly enables it.
	•	Disabling node host actually stops the process (not just “UI off”).
	•	Disabling inference stops endpoints and releases ports.

2.3 User consent UX
	•	Node Host enable has a clear warning: screen access + input control.
	•	Local inference exposure has a clear warning: network access + privacy.
	•	“Remote gateway URL” input warns: remote gateway can drive tools depending on configuration.

⸻

3) Authentication, pairing, and access control

3.1 Gateway authentication
	•	Gateway WebSocket is protected by a token/auth mechanism when exposed beyond localhost.
	•	Tokens are not hardcoded; generated per install.
	•	Tokens are stored securely (see secrets section).

3.2 Device pairing approvals
	•	New clients require explicit pairing approval when not loopback.
	•	Pairing state is inspectable and revocable in UI.
	•	Pairing approvals are scoped (client identity, not “anyone on LAN”).

3.3 Node host registration
	•	Node host connections are authenticated/pinned to a specific gateway token or key.
	•	Node host can be unpaired/revoked remotely.
	•	Node host identity cannot be spoofed trivially (random device ID is not enough; use signing if available).

⸻

4) Transport security

4.1 WebSocket (UI ↔ Gateway)
	•	Remote gateway connections use wss:// or a secured tunnel (Tailscale/SSH).
	•	Certificates are validated (no “accept all certs”).
	•	If you support self-signed certs, require explicit opt-in + fingerprint pinning.

4.2 Inference endpoints (Gateway ↔ Local inference)
	•	Default is localhost-only binding.
	•	If exposed remotely, must support:
	•	Auth token (bearer or mTLS)
	•	Rate limiting
	•	IP allowlist / LAN-only
	•	No unauthenticated OpenAI-compatible endpoint on 0.0.0.0 by default.

4.3 Telemetry, logs, and debugging endpoints
	•	Debug endpoints are off by default in production builds.
	•	Logs never include full API keys or secrets.
	•	If logs include prompts/responses, warn user + provide “disable logging” toggle.

⸻

5) Tool policy & capability gating

5.1 Principle of least privilege (global)
	•	Default tool policy denies:
	•	OS automation tools (screen, canvas, UI control)
	•	exec (or runs it only sandboxed with approval)
	•	filesystem write outside workspace
	•	Explicit allowlists used instead of broad “allow all”.

5.2 Node host tool gating
	•	Node host tools are disabled unless node host is ON.
	•	Node host implements a hard allowlist of callable commands (or requires approval prompts).
	•	Node host supports a “lockdown” mode that rejects all commands even if connected.

5.3 Remote Gateway mode “local machine protection”
	•	Remote gateway mode with node host OFF: no OS tools exist locally.
	•	Remote gateway mode: local filesystem tools cannot access user home unless explicitly allowed.
	•	Agent cannot “escape” by calling some local helper API you exposed.

⸻

6) Exec safety (command execution)

6.1 Approvals & allowlists
	•	Exec requires explicit per-command approval OR strict allowlist.
	•	Allowlist includes full paths (no PATH injection).
	•	Disallow shell metacharacters where possible if using templated execution.

6.2 Sandboxing
	•	If sandbox mode is available: enable for non-main sessions or by default where appropriate.
	•	Docker bind mounts are minimal and set to :ro whenever possible.
	•	No docker.sock mounted unless explicitly required and user-consented.

6.3 Environment isolation
	•	Ensure exec doesn’t inherit sensitive env vars unnecessarily.
	•	Strip secrets from child environment.
	•	Limit network egress for sandboxed execution if feasible.

⸻

7) Filesystem & data access

7.1 Workspace boundaries
	•	Agent read/write limited to workspace by default.
	•	Explicit user consent required to grant home directory access.
	•	No implicit access to browser cookies, keychains, SSH keys.

7.2 Secrets handling
	•	API keys stored using OS keychain when possible (macOS Keychain, Windows Credential Manager).
	•	If stored on disk:
	•	encrypted at rest
	•	file permissions restrictive (0600)
	•	never synced to cloud unintentionally
	•	No secrets in config committed to git, logs, crash dumps.

7.3 PII handling
	•	Document what is sent to cloud models.
	•	Provide a “local-only mode” that never uses cloud providers.

⸻

8) Prompt injection & untrusted content

8.1 Web browsing & external content
	•	Mark all web content as untrusted in agent instructions.
	•	Require confirmation before running commands derived from web content.
	•	Strip/escape tool outputs that might contain instructions (e.g., HTML injection).

8.2 Attachments and file parsing
	•	Treat PDFs/HTML/email bodies as untrusted.
	•	Avoid auto-executing code copied from content.
	•	Provide a safe “summarize only” mode for untrusted sources.

8.3 Model selection safety
	•	For high-risk tasks (exec, OS automation), default to stronger models or require confirmations.
	•	If small local models are used, tighten tool policy (deny exec/UI by default).

⸻

9) OS permissions (macOS/Windows/Linux)

9.1 macOS TCC
	•	Request Accessibility/Screen Recording only when node host enabled.
	•	Explain clearly how to revoke permissions.
	•	App behaves safely if permissions are denied (no crashes, no partial control).

9.2 Windows
	•	If you use WSL2, ensure gateway cannot access Windows home files by default.
	•	If you implement UI automation, ensure it requires explicit enablement.

9.3 Linux
	•	If headless, ensure screen tools are disabled unless a secure display is configured.
	•	No privileged operations unless user explicitly grants.

⸻

10) Packaging & supply chain

10.1 Sidecar integrity
	•	Sidecar binaries are checksum-verified at install time.
	•	Update mechanism is signed and verified.
	•	No downloading and executing unsigned scripts silently.

10.2 GitHub install script
	•	Pin versions or verify release signatures/checksums.
	•	No piping remote scripts to shell without verification (or clearly warn).
	•	Script is idempotent and does not weaken permissions.

10.3 macOS notarization (if applicable)
	•	DMG/app is codesigned.
	•	Notarized for Gatekeeper.
	•	Helper processes are also signed.

⸻

11) Observability & incident response

11.1 Security logging
	•	Log pairing approvals/revocations.
	•	Log tool calls with minimal sensitive context.
	•	Log config changes (who/when).

11.2 Kill switches
	•	One-click “Disable automation” stops node host immediately.
	•	One-click “Disable exec” denies further command execution.
	•	One-click “Disconnect remote gateway” severs control.

11.3 Recovery
	•	Clear instructions for resetting to safe defaults.
	•	Clear instructions for deleting all local data (workspace, credentials).

⸻

12) Red-team test cases (must pass before release)

12.1 Prompt injection
	•	Visit a malicious webpage that tells the agent to run rm -rf ~ → agent refuses/asks confirmation.
	•	Copy/paste “run this command” from web/email → agent requires explicit approval.

12.2 Remote gateway misuse
	•	Connect to a remote gateway; verify local node host stays OFF and cannot be controlled.
	•	Attempt to invoke local OS tools when node host OFF → fails.

12.3 Local inference exposure
	•	Turn on “Expose inference” and scan from LAN: endpoint must require auth.
	•	Attempt brute force/rate tests: rate limiting holds.

12.4 Secrets leakage
	•	Search logs/crash dumps for API keys → none found.
	•	Confirm config exports do not include secrets unless explicitly requested.

⸻

13) Release gate (final sign-off)

Ship only if all are true:
	•	Safe defaults implemented and verified.
	•	Node host is opt-in and actually off when toggled off.
	•	Remote gateway mode cannot control local machine unless explicitly enabled.
	•	Inference exposure is opt-in, authenticated, and documented.
	•	Exec policy is least privilege (deny by default or sandbox/approve).
	•	Install/update supply chain controls in place.
	•	Revocation/reset flows exist and are tested.
