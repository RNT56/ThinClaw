# AgentEnv

`src/agent/env/` is the Phase 1 eval/SFT environment layer. It wraps the normal ThinClaw agent loop, so environment rollouts reuse existing prompt assembly, tool execution, trajectory logging, learning review, and `AgentRunArtifact` storage.

## Core Types

- `AgentEnv`: async trait with `reset`, `step`, `score`, `is_terminal`, and `export_trajectory`.
- `AgentLoopEnv`: concrete environment backed by `Agent::handle_message_external`.
- `EnvRunner`: scripted evaluator, SFT JSONL collector, and OpenAI-compatible local serving wrapper.
- `Trajectory`: serializable rollout record with steps, rewards, and metadata.

## Phase 1 Modes

- `evaluate`: run scripted episodes and persist `agent_env` artifacts.
- `collect_sft_jsonl`: keep positive trajectories and write chat-format JSONL.
- `serve_openai_compatible`: expose `/v1/chat/completions` for eval harnesses that speak OpenAI-compatible chat.

Phase 2 is deliberately deferred: exact token IDs, logprobs, and managed RL server integration should build on top of stable trajectory collection rather than landing in the first tranche.

## Rewarding

The default `AgentLoopEnv` uses a simple heuristic reward: non-empty successful responses score high, explicit errors score low, and empty responses score zero. Benchmarks should replace or post-process this with task-specific scoring.
