# ThinClaw Desktop Documentation Index

Last updated: 2026-07-13

This directory is the current contract surface for ThinClaw Desktop. Keep these files current when backend contracts, runtime ownership, remote gateway behavior, secrets, setup, or release acceptance changes.

## Contract And Runtime

- [Runtime Boundaries](runtime-boundaries.md): the two-system Desktop architecture: Direct AI Workbench vs ThinClaw Agent Cockpit, shared services, state ownership, and iOS implications.
- [Bridge Contract](bridge-contract.md): stable `thinclaw_*` Tauri commands, `thinclaw-event`, event routing metadata, generated bindings, and local/remote command rules.
- [Runtime Parity Checklist](runtime-parity-checklist.md): desktop runtime parity against root ThinClaw behavior, including fixture and release-operator criteria.
- [Remote Gateway Route Matrix](remote-gateway-route-matrix.md): every desktop command group, its local behavior, remote route, and explicit unavailable behavior.
- [Security Posture](security-posture.md): sources, retention, mode boundaries, and deliberate limits of the read-only Security panel.
- [Bridge Threat Model](threat-model.md): untrusted runtime output, React rendering, remote bearer transport, profile-token storage, and SSH deployment controls.
- [RAG And Embedding Compatibility](rag-inference-compatibility.md): provider defaults, query/document semantics, live dimension authority, index migration, and pinned reranker artifacts.
- [Desktop Accessibility Contract](accessibility.md): keyboard/focus behavior, assistive semantics, contrast, reduced motion, and regression coverage.

## Operations

- [Environment Requirements](env-requirements.md): supported toolchain, setup commands, env vars, generated runtime paths, and final gate commands.
- [Secrets Policy](secrets-policy.md): ThinClaw key naming, legacy fallback, grant checks, injection rules, and remote-mode secrecy constraints.
- [Manual Smoke Checklist](manual-smoke-checklist.md): repeatable local and remote acceptance pass for release candidates.
- [Packaging And Platform Readiness](packaging-platform-readiness.md): macOS packaging checks and remaining host prerequisites.

## Release Constraints

- [External Release Prerequisites](external-release-prerequisites.md): release-operator inputs that cannot be proven by committed fixtures.
