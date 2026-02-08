Below is a refined engineering spec tailored to Scrappy (Tauri + Rust backend, React frontend) using llama.cpp + rig for LLM, sd.cpp for diffusion, and an existing RAG pipeline. It’s written to plug into an existing codebase and focuses on tool-aware conversational orchestration with clean module boundaries, strict data contracts, and UI integration.

⸻

Scrappy Tool-Aware Conversational Agent Spec

0) Goals

Primary
	•	Provide a general conversational agent that can decide when to use tools (RAG, web search, image generation, calculators, file parsing, etc.) and respond naturally.
	•	Support open-source local models (via llama.cpp) and local image generation (sd.cpp).
	•	Ensure grounded answers when using RAG/web with citations and reliable fallbacks.

Non-goals (v1)
	•	Multi-user sync, cloud inference, autonomous background execution.
	•	Full “agentic workflow” with complex multi-step planning beyond a small controlled loop.

⸻

1) Architectural Overview

1.1 High-level flow
	1.	User sends message in React UI
	2.	Tauri command → Rust “Orchestrator”
	3.	Orchestrator runs:
	•	Safety & intent checks
	•	Tool router → produces a structured Tool Plan
	•	Executes tools (RAG / web / image / etc.)
	•	Builds grounded context pack
	•	LLM “Writer” generates final response
	•	Optional verifier pass (cheap)
	4.	Stream response + tool events back to UI

1.2 Backend modules (Rust)
	•	orchestrator/
Owns the agent loop + state machine per conversation.
	•	llm/
Wrapper around rig + llama.cpp, supports streaming tokens.
	•	tools/
Tool registry + implementations:
	•	rag_tool
	•	web_tool (optional if you have a provider)
	•	image_tool (sd.cpp)
	•	calc_tool, file_tool (optional)
	•	rag/
Existing RAG entrypoints (retrieve, rerank, cite, pack).
	•	safety/
Policy checks, redaction, refusal templates.
	•	telemetry/
Logs, traces, per-turn artifacts.

1.3 Frontend modules (React)
	•	ChatView + MessageList + Composer
	•	ToolActivityPanel (shows: “Searching…”, “Retrieving docs…”, “Generating image…”)
	•	CitationsViewer (doc/web citations in assistant messages)
	•	ModelSettings (model selection, temperature, context window)
	•	RAGSettings (topK, reranker toggle, collection filters)
	•	ImageStudio (prompt refinement, generation history, variants)

⸻

2) Conversation & State Model

2.1 Conversation state (Rust)

Each conversation has:
	•	conversation_id: Uuid
	•	messages: Vec<Message> (thin history or summarized)
	•	memory: ConversationMemory
	•	running_summary: String (updated periodically)
	•	pinned_facts: Vec<String> (user preferences, if enabled)
	•	last_tool_artifacts: Vec<ToolArtifact> (tool results, citations)
	•	settings: ConversationSettings (model, tool policy, RAG config)

2.2 Message schema

{
  "id": "uuid",
  "role": "user|assistant|tool",
  "content": "string",
  "timestamp": "iso8601",
  "attachments": [
    {"type":"image|pdf|text","uri":"scrappy://...","meta":{}}
  ],
  "citations": [
    {"kind":"rag|web","source_id":"string","title":"string","loc":"string","confidence":0.0}
  ]
}


⸻

3) Agent Loop Specification

3.1 Loop phases (deterministic state machine)

Phase A — Analyze
	•	Detect:
	•	intent category (chat / info / creative / image / troubleshooting)
	•	freshness requirement (e.g., “latest/today/current”)
	•	grounding requirement (“according to our docs…”, “policy says…”, “in the uploaded pdf…”)
	•	tool eligibility (attachments present? web enabled? image tool enabled?)
	•	safety flags (medical/legal/financial, disallowed content)

Phase B — Route
	•	Produce a strict Tool Plan JSON (see 4.1).
	•	Router can be:
	•	rule-based + short LLM router prompt fallback (recommended)
	•	or LLM-only router with JSON output + schema validation

Phase C — Execute Tools
	•	For each tool step:
	•	emit UI tool event tool_started
	•	call tool implementation
	•	store tool artifacts and citations
	•	emit tool_finished with summary + artifact ids

Phase D — Write
	•	Compose a context pack:
	•	conversation summary + last N turns
	•	selected tool outputs (short, chunked, cited)
	•	constraints (tone, “must cite if web/RAG used”)
	•	Call writer LLM with streaming tokens

Phase E — Verify (optional)
	•	Lightweight check:
	•	if tool outputs exist → response must include citations
	•	if user asked “latest/current” and web not used → force “can’t verify locally” message
	•	verify no contradictions with tool artifacts (simple heuristic)

Phase F — Respond
	•	Return assistant message with citations + tool artifacts + optional image URIs

⸻

4) Tool Router Spec

4.1 Tool Plan schema (authoritative)

{
  "decision": "no_tool|rag|web|rag+web|image|clarify",
  "reason": "string",
  "steps": [
    {
      "tool": "rag|web|image|calc|file",
      "input": {},
      "priority": 1
    }
  ],
  "response_style": "brief|normal|detailed",
  "safety_mode": "default|high",
  "constraints": {
    "require_citations": true,
    "max_tool_steps": 3,
    "web_recency_days": 14
  }
}

4.2 Routing rules (v1)

Hard rules (deterministic):
	•	If user asks to generate/edit an image → decision=image
	•	If user references internal docs / KB / policy / uploaded files → include rag (or file then rag)
	•	If user asks for fresh/current/latest AND web tool enabled → include web
	•	If web is disabled but freshness required → respond with limitation + offer best-effort using local KB

Soft rules (LLM router or heuristics):
	•	If user asks factual question and confidence low → prefer rag
	•	If user asks comparisons requiring latest info (prices, versions) → prefer web

4.3 Router implementation detail
	•	Implement RouteEngine:
	•	route_deterministic(input) -> Option<ToolPlan>
	•	else route_llm_json(input) -> ToolPlan (validated)
	•	Always validate against schema; if invalid, fall back to safe deterministic plan.

⸻

5) Tool Contracts

5.1 Tool trait (Rust)
	•	Tool::name() -> &'static str
	•	Tool::run(ctx: ToolContext, input: serde_json::Value) -> ToolResult

ToolResult standard:

{
  "ok": true,
  "summary": "string (short for UI)",
  "data": {},
  "citations": [
    {"source_id":"...","title":"...","loc":"...","confidence":0.0}
  ],
  "artifacts": [
    {"kind":"image|text|json","uri":"scrappy://...","meta":{}}
  ],
  "timings_ms": {"total": 1234}
}

5.2 RAG Tool (tools/rag_tool)

Input:

{
  "query": "string",
  "top_k": 8,
  "filters": {"collection":"default","tags":["..."]},
  "rerank": true
}

Output:
	•	data.passages[] with doc_id, chunk_id, text, score
	•	citations for each passage
	•	summary for UI: “Retrieved 8 passages from 3 documents”

RAG packing rule: never dump all text into LLM. Pass only top passages + short extracts.

5.3 Web Tool (tools/web_tool) (if enabled)

Input:

{
  "query": "string",
  "recency_days": 14,
  "domains_allowlist": [],
  "max_results": 5
}

Output:
	•	data.results[]: title, snippet, url, published_at (if available)
	•	citations must include url + timestamp when possible

5.4 Image Tool (tools/image_tool) using sd.cpp

Input:

{
  "prompt": "string",
  "negative_prompt": "string",
  "width": 768,
  "height": 768,
  "steps": 30,
  "cfg_scale": 7.0,
  "seed": 42,
  "n": 1
}

Output:
	•	artifacts contain scrappy://images/<id>.png
	•	summary: “Generated 1 image (768×768)”

Policy: if request is disallowed → refuse before calling sd.cpp.

⸻

6) LLM Prompting Strategy (rig + llama.cpp)

6.1 Roles
	•	Router prompt: very short, strict JSON
	•	Writer prompt: normal assistant behavior + tool-grounding rules
	•	Verifier prompt (optional): checklist JSON (“ok”: true/false, “issues”: [])

6.2 Writer guardrails (must be enforced)
	•	If RAG or Web used → include citations.
	•	If user asked for “current/latest” without web → say you can’t verify up-to-date, offer to use web if enabled.
	•	Never fabricate document titles/sections; only cite provided citations.

6.3 Context packing constraints (important for 3B models)
	•	Use:
	•	conversation summary + last 6 turns
	•	max ~1–2k tokens of retrieved text total
	•	prefer bullet extracts + short quotes
	•	Ensure citations are adjacent to the claims they support (or grouped per paragraph).

⸻

7) Frontend UX Requirements

7.1 Streaming + tool activity
	•	Use Tauri event channel to emit:
	•	assistant_token
	•	tool_started { tool, summary }
	•	tool_finished { tool, summary, artifacts }
	•	assistant_done { message, citations }

7.2 Message rendering
	•	Assistant messages support:
	•	markdown
	•	inline citations (click opens citation panel)
	•	tool artifacts (image cards, doc snippet cards)

7.3 Settings UI
	•	Toggle tools:
	•	Web enabled/disabled
	•	RAG enabled/disabled + collection selector
	•	Image enabled/disabled
	•	Model selection:
	•	LLM model path, ctx size, temp, top_p
	•	sd.cpp model path, default resolution/steps

⸻

8) Safety & Policy

8.1 Enforcement points
	•	Before routing: disallowed content + sensitive requests
	•	Before tool run: tool-specific constraints
	•	After writing: refusal sanitization, remove sensitive leakage

8.2 Domain “high safety mode”

If detected medical/legal/financial + user asks for instructions:
	•	Force grounded response (RAG/web if available)
	•	Add “informational” framing + suggest professional help where appropriate
	•	No definitive claims without sources

⸻

9) Telemetry & Debugging

Per turn, store:
	•	Router input features + Tool Plan
	•	Tool calls + timings
	•	Retrieved doc ids and scores
	•	Final prompt token counts (approx)
	•	Streaming completion stats

Expose a “Debug Drawer” in UI:
	•	Tool Plan JSON
	•	Retrieved passages (collapsed)
	•	Final context pack (collapsed)
	•	Verifier result

⸻

10) Testing & Acceptance Criteria

10.1 Required behaviors
	•	“latest/current/today” prompts:
	•	if web enabled → web tool called
	•	if web disabled → explicit limitation in response
	•	“According to our policy/docs” prompts:
	•	RAG tool called
	•	citations shown
	•	Image requests:
	•	image tool called
	•	image rendered in UI
	•	Tool failure:
	•	graceful fallback: “I couldn’t fetch X; here’s what I can do…”

10.2 Golden tests (recommended)
	•	A set of fixed conversations + snapshots of:
	•	Tool Plan
	•	tool calls invoked
	•	presence of citations
	•	response structure

⸻

Implementation Notes (Tauri Integration)

Tauri Commands (suggested)
	•	chat_send_message(conversation_id, user_message, attachments, settings) -> stream
	•	conversation_create() -> conversation_id
	•	conversation_list()
	•	conversation_get(conversation_id)
	•	rag_reindex() (if applicable)
	•	image_generate(prompt, params) (optional direct entry)

Event names
	•	scrappy://chat/token
	•	scrappy://chat/tool_started
	•	scrappy://chat/tool_finished
	•	scrappy://chat/done
	•	scrappy://chat/error

⸻

