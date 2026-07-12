# Prompt System V2

ThinClaw constructs prompts through the provider-neutral contract in
`thinclaw-llm-core`. Every detected prompt-bearing production file has one
exact owner in `docs/prompt-registry.json`; broad directory prefixes are
documentation only and never satisfy `scripts/ci/prompt_audit.py`.

The interactive dispatcher has one final authority path. `PromptStack`
composes readable policy sections, converts them to a required typed segment,
and `PromptCompiler` compiles that policy together with workspace identity,
skills, runtime configuration, and untrusted evidence once per actual LLM
request. There is no second unbudgeted conversation-prompt append step.

## Authority

Only immutable policy and trusted configuration may enter a system preamble.
User instructions remain user messages. Memory, retrieval, web content, tool
results, diffs, transcripts, repository text, and reference-model answers are
untrusted evidence and must use `ChatMessage::untrusted_context` or an
`UntrustedData` `PromptSegment`.

## Budgets and telemetry

`PromptBudget` reserves output capacity, tool schemas, history, and a safety
margin before optional prompt material is selected. Required policy is never
silently truncated. `CompiledPrompt` exposes content-free manifest records,
stable and turn hashes, a manifest digest, and approximate token totals. Raw
prompt or evidence content must not be written to telemetry.

Interactive compilation uses the tool schemas that survive routing for the
actual turn, the current conversation history, and the current output cap.
Duplicate segment IDs and required-policy overflow fail closed before a
provider request is sent. The exact content-free manifest is persisted after
compilation, replacing the source-graph preview used by shadow mode.

Code-owned per-turn directives use `ChatMessage::immutable_policy`; trusted
runtime/configuration overlays use `ChatMessage::trusted_prompt`. Prompt V2
consumes these messages into the compiler and removes their standalone system
roles before transport. Any untyped system message is demoted to user-role
untrusted evidence rather than gaining authority implicitly.

## Production rollout

V2 is the default for new sessions. Operators can explicitly select `shadow`
or `legacy` during the one-release compatibility window. A session with a
frozen pre-V2 prompt snapshot remains on the legacy contract until its session
boundary; sessions already marked `v2` likewise remain pinned to V2.

## Machine output

Machine-consumed responses use exact typed JSON. Parsers reject extra prose,
unknown fields, malformed output, invalid ranges, and ambiguous sentinels.
Security triage is restricted to `deny` or `escalate`; only deterministic
policy or a human can approve execution.

## Adding or changing a prompt

Update the exact `prompt_paths` owner and contract metadata, construct
interactive authority through `PromptStack` plus the shared compiler, add
adversarial parser/authority tests, and run:

```sh
python3 scripts/ci/prompt_audit.py
cargo test -p thinclaw-llm-core prompt_contract
```
