Below is a complete, implementation-ready specification you can drop into your repo as e.g.
ARCHITECTURE.md or DESKTOP_COMPANION_SPEC.md.

It is written as a downloadable Markdown spec, not marketing docs, and assumes an engineering audience.

⸻

Custom Desktop Companion App – OpenClaw Architecture & Implementation Spec

0. Purpose & Scope

This document specifies the target architecture and implementation details for a custom desktop companion application that:
	•	Embeds or connects to an OpenClaw gateway + agent
	•	Optionally provides a node host (OS automation, screen, canvas, UI control)
	•	Optionally provides local inference (llama.cpp, sd.cpp)
	•	Supports local gateway and remote gateway modes
	•	Safely supports cloud model fallback
	•	Is distributable via:
	•	GitHub install script
	•	macOS .dmg
	•	Windows .exe (future)

This spec is designed to be fully implementable, not conceptual.

⸻

1. Core Design Principles
	1.	Gateway ≠ Node Host ≠ UI
	•	These are separate responsibilities and must remain separable.
	2.	Safe by default
	•	No OS automation unless explicitly enabled.
	•	Remote gateway mode never touches the local machine unless allowed.
	3.	Explicit capability boundaries
	•	Inference ≠ Automation ≠ Agent location.
	4.	Restart is acceptable
	•	Node host enable/disable may require restart.
	5.	One app, multiple deployment topologies
	•	Same binary supports all modes via configuration.

⸻

2. High-Level Component Overview

2.1 Desktop App (Tauri)

Always present
	•	React UI
	•	Settings & mode switching
	•	Process manager for sidecars
	•	WebSocket client for gateway
	•	Optional local inference server

Never assumes OS automation

⸻

2.2 OpenClaw Gateway (Node.js sidecar)

Responsibilities
	•	Agent runtime
	•	Tool routing & policy
	•	Model provider selection
	•	Session & memory handling
	•	WebSocket API for clients

Deployment
	•	Local gateway: bundled & launched by app
	•	Remote gateway: external, app connects as client

⸻

2.3 Node Host (Optional)

Responsibilities
	•	OS-level tools:
	•	screen capture / recording
	•	canvas
	•	accessibility / UI automation
	•	system.run
	•	Owns OS permissions (macOS TCC)

Deployment
	•	Optional
	•	Runs on whichever machine should be controlled
	•	Not required for gateway operation

⸻

2.4 Inference Services (Optional)

Local
	•	llama.cpp (OpenAI-compatible endpoint)
	•	sd.cpp diffusion endpoint

Remote / Cloud
	•	OpenAI / Anthropic / others
	•	Configured on the gateway

⸻

3. Operating Modes (Authoritative Matrix)

Mode	Gateway	Node Host	Local Inference	OS Control	Typical Use
A1	Local	OFF	OFF	❌	Chat UI only
A2	Local	OFF	ON	❌	Local LLM app
A3	Local	ON	ON/OFF	✅	Full local assistant
B1	Remote	OFF	OFF	❌	Safe client
B2	Remote	OFF	ON (exposed)	❌	Remote agent, local compute
B3	Remote	ON	ON/OFF	✅	Remote agent controls this PC

Default recommended modes
	•	Install default: A2
	•	Remote gateway default: B1

⸻

4. Runtime Toggles (UI Requirements)

4.1 Gateway Mode
	•	Local Gateway
	•	Remote Gateway (URL)

Behavior
	•	Switching modes:
	•	Stops/starts gateway sidecar
	•	Reconnects WS client
	•	Restart acceptable

⸻

4.2 Node Host / OS Automation
	•	Enable OS Automation (OFF by default)

When enabled
	•	Starts node host process
	•	Requests OS permissions
	•	Registers node with gateway

When disabled
	•	Node host process not running
	•	No OS tools available
	•	Permissions remain granted but unused

⸻

4.3 Local Inference
	•	Enable Local Inference

Sub-toggle (Remote Gateway only)
	•	Expose local inference to remote gateway

Security
	•	Must support:
	•	localhost-only binding
	•	optional auth token
	•	explicit user confirmation

⸻

5. Process Model

5.1 Local Gateway Mode

[Tauri App]
 ├─ UI
 ├─ OpenClaw Gateway (Node.js sidecar)
 ├─ Local Inference (optional)
 └─ Node Host (optional)

5.2 Remote Gateway Mode

[Tauri App]
 ├─ UI (WS client)
 ├─ Local Inference (optional)
 └─ Node Host (optional)

[Remote Machine]
 ├─ OpenClaw Gateway + Agent
 ├─ Cloud / Remote Inference
 └─ Remote Node Host (optional)


⸻

6. Node Host Implementation Guidance

6.1 Node Host is NOT the Gateway

The node host:
	•	Connects to gateway as role=node
	•	Exposes a fixed set of tools
	•	Never runs inference
	•	Never routes messages

6.2 Recommended Implementation

macOS
	•	Separate helper process or embedded Tauri module
	•	Owns:
	•	Accessibility permission
	•	Screen Recording permission
	•	Exposes:
	•	screenshot / screen stream
	•	UI automation
	•	filesystem (scoped)

Windows
	•	Phase 1: no node host (safe)
	•	Phase 2: dedicated Windows automation helper

⸻

7. Tool Policy (Critical)

7.1 Defaults

{
  "tools": {
    "allow": ["group:messaging", "group:fs", "group:runtime"],
    "deny": ["group:ui"]
  }
}

7.2 When Node Host Enabled

{
  "tools": {
    "allow": ["group:ui", "group:runtime", "group:fs"]
  }
}

7.3 Remote Gateway Safety Rule

If local node host is disabled, never forward exec or UI tools to local machine.

⸻

8. Model Provider Strategy

8.1 Local Gateway Mode

Priority order:
	1.	Local inference (if enabled)
	2.	Cloud providers (configured)

8.2 Remote Gateway Mode

Priority order:
	1.	Cloud providers
	2.	Remote inference on gateway
	3.	Optional: exposed local inference (explicit opt-in)

8.3 Failure Handling
	•	Gateway must support provider fallback
	•	Inference errors must not crash agent loop

⸻

9. Configuration Model

9.1 App-side State (example)

{
  "gatewayMode": "local | remote",
  "remoteGatewayUrl": "wss://...",
  "nodeHostEnabled": false,
  "localInferenceEnabled": true,
  "exposeInference": false
}

9.2 Gateway Config (generated)
	•	Written by app
	•	Passed via env or file
	•	Regenerated on mode switch

⸻

10. Installation & Distribution

10.1 GitHub Install Script

Responsibilities
	•	Detect OS
	•	Download app release
	•	Install dependencies (Node, optional Docker)
	•	Optionally install node host helper

Profiles
	•	--safe
	•	--automation

⸻

10.2 macOS DMG

Bundled
	•	Tauri app
	•	Node.js sidecar binary
	•	Optional helper binary

First-launch flow
	1.	Select gateway mode
	2.	Select automation (node host)
	3.	Request permissions only if needed

⸻

10.3 Windows EXE (Future)

Recommended
	•	Gateway in WSL2
	•	UI native
	•	Node host later

⸻

11. Security Guarantees (Must Hold)
	•	Remote gateway mode never controls local OS unless node host enabled
	•	Node host disabled = no automation possible
	•	Inference exposure requires explicit user opt-in
	•	Tool policy enforced at gateway

⸻

12. Non-Goals (Explicit)
	•	Automatic OS control without consent
	•	Silent permission escalation
	•	Implicit inference sharing
	•	Single hard-coded deployment topology

⸻

13. Implementation Checklist

MVP
	•	Gateway local/remote switch
	•	Local inference on/off
	•	Tool policy enforcement
	•	Restart-based toggles

Automation Add-On
	•	Node host helper
	•	Permission UX
	•	Safe defaults

⸻

14. Final Architectural Truth

Gateway = brain
Node host = body
Inference = compute
UI = interface

They are independent, optional, and composable — and your app should keep them that way.

⸻

End of specification.

⸻
