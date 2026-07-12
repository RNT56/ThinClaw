# Prompt System V2

ThinClaw constructs prompts through the provider-neutral contract in
`thinclaw-llm-core`. Every prompt-bearing production surface is owned by an
entry in `docs/prompt-registry.json` and checked by
`scripts/ci/prompt_audit.py`.

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

Update the registry owner and contract metadata, construct authority through
the shared compiler, add adversarial parser/authority tests, and run:

```sh
python3 scripts/ci/prompt_audit.py
cargo test -p thinclaw-llm-core prompt_contract
```
