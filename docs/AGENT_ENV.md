# AgentEnv

`src/agent/env/` is the Phase 1 eval/SFT environment layer. It wraps the normal ThinClaw agent loop, so environment rollouts reuse existing prompt assembly, tool execution, trajectory logging, learning review, and `AgentRunArtifact` storage.

## Core Types

- `AgentEnv`: async trait with `reset`, `step`, `score`, `is_terminal`, and `export_trajectory`.
- `AgentLoopEnv`: concrete environment backed by `Agent::handle_message_external`.
- `TerminalBenchEnv`: command-case benchmark environment with stdout/exit-code scoring.
- `SkillBenchEnv`: skill-content benchmark environment with deterministic readiness checks.
- `EnvRunner`: scripted evaluator, SFT JSONL collector, and OpenAI-compatible local serving wrapper.
- `Trajectory`: serializable rollout record with steps, rewards, metadata, and optional token/logprob capture capability flags.

## Phase 1 Modes

- `evaluate`: run scripted episodes and persist `agent_env` artifacts.
- `collect_sft_jsonl`: keep positive trajectories and write chat-format JSONL.
- `serve_openai_compatible`: expose `/v1/chat/completions` for eval harnesses that speak OpenAI-compatible chat.
- Research campaigns: use an `agent_env` runner with `backend_config.benchmark` set to `terminal_bench` or `skill_bench`; the completion path persists trajectory JSON plus normal experiment metrics.

Exact token IDs and logprobs are captured only when the active provider can supply them. Unsupported providers keep the trajectory fields present but set capability flags false instead of emitting synthetic exact-token data.

## Rewarding

The default `AgentLoopEnv` uses a simple heuristic reward: non-empty successful responses score high, explicit errors score low, and empty responses score zero. Benchmarks should replace or post-process this with task-specific scoring.
