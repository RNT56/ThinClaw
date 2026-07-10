#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v rg >/dev/null 2>&1; then
  echo "error: ripgrep (rg) is required" >&2
  exit 1
fi

echo "# ThinClaw Loop Inventory"
echo
echo "## Core loop files"
for path in \
  "src/agent/dispatcher/loop.rs" \
  "src/agent/worker.rs" \
  "src/agent/subagent_executor/mod.rs" \
  "src/agent/routine_engine.rs" \
  "src/agent/outcomes.rs" \
  "src/agent/self_repair.rs" \
  "src/agent/job_monitor.rs" \
  "src/agent/agent_loop/mod.rs" \
  "src/repo_projects/supervisor.rs" \
  "src/repo_projects/github/transport.rs" \
  "crates/thinclaw-agent/src/worker_runtime.rs" \
  "crates/thinclaw-agent/src/dispatcher_policy.rs" \
  "crates/thinclaw-agent/src/subagent.rs" \
  "crates/thinclaw-agent/src/loop_control.rs"; do
  if [[ -f "$path" ]]; then
    printf "%6s  %s\n" "$(wc -l < "$path" | tr -d ' ')" "$path"
  fi
done

echo
echo "## Spawn and task ownership sites"
rg -n "tokio::spawn|JoinHandle|JoinSet|spawn_[a-z_]*\\(" src crates --glob '*.rs'

echo
echo "## Long-running receiver and interval loops"
rg -n "loop \\{|while let Some|while let Ok|tokio::time::interval|recv\\(\\)\\.await|watchdog|ticker" \
  src/agent src/repo_projects crates/thinclaw-agent --glob '*.rs'

echo
echo "## Shared loop-control usage"
rg -n "LoopBudget|LoopKind|LoopRunContext|LoopRunSummary|LoopStopReason|LoopRetryPolicy" \
  src crates --glob '*.rs'

echo
echo "## Shutdown and cancellation ownership"
rg -n "shutdown_rx|shutdown_tx|abort_all|drain_or_abort|JoinSet::|\.shutdown\(\)" \
  src/agent src/repo_projects crates/thinclaw-agent --glob '*.rs'
