# ThinClaw Documentation Audit

This workspace coordinates the documentation reconciliation effort for ThinClaw.

## Goals

- Compare current documentation against the actual codebase
- Identify contradictions, stale claims, duplication, and missing coverage
- Define canonical homes for each concept
- Produce a rewrite order that favors high signal and ThinClaw-specific detail

## Working Rules

- Code is the primary source of truth
- Module spec docs outrank marketing or overview docs
- Historical migration material stays archived unless explicitly referenced as history
- Every claim should point to evidence in code or a verified canonical doc
- Avoid generic AI-agent filler; keep prose ThinClaw-specific

## Core Outputs

- `truth-map.md` records claims, evidence, and actions
- `contradictions.md` records direct doc-vs-doc or doc-vs-code conflicts
- `source-of-truth-matrix.md` assigns canonical homes by topic
- `rewrite-order.md` defines execution order and priority
- `reports/` contains domain-specific audit reports from parallel agents

## Domain Reports

- `reports/repo-cartography.md`
- `reports/runtime-architecture.md`
- `reports/channels-and-delivery.md`
- `reports/tools-extensions-mcp.md`
- `reports/setup-config-deployment.md`
- `reports/security-safety-trust.md`
- `reports/cli-web-operator-surface.md`

## Status Model

- `accurate`
- `partial`
- `stale`
- `contradicted`
- `archive`

## Action Model

- `keep`
- `rewrite`
- `split`
- `merge`
- `archive`
- `delete`
