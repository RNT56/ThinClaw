# Observability

ThinClaw records lifecycle events and metrics through a pluggable `Observer`
trait (`src/observability/`). One backend is selected at startup.

## Selecting a backend

Set the backend via the onboarding wizard (Observability step) or the
`OBSERVABILITY_BACKEND` environment variable:

| Value | Behavior |
|---|---|
| `log` (default) | Emits structured events via `tracing`, alongside normal logs. Zero extra dependencies. |
| `none` / `noop` | Discards everything at zero cost. |
| `prometheus` | Records counters/histograms/gauges into a Prometheus registry and exposes them at `GET /metrics` on the gateway. Also still visible in logs is not implied — Prometheus is metrics-only; keep `RUST_LOG` for logs. |

Unknown values fall back to `noop`.

## The `/metrics` endpoint

When `OBSERVABILITY_BACKEND=prometheus`, the gateway serves Prometheus
text-exposition format at `GET /metrics` (no authentication — the scraper
standard). All series are prefixed `thinclaw_`.

- Auth posture: **public, unauthenticated** by design (Prometheus scrapers do
  not send bearer tokens). The endpoint exposes only aggregate operational
  counters — no end-user message content, so no PII exposure. Most label values
  (`provider`, `model`, `channel`, `component`, `direction`) are low-cardinality
  operator configuration. **Caveat:** the `tool` label for MCP tools is derived
  from an MCP server's advertised tool names — operator-trusted but not
  operator-authored — so a compromised MCP server could drive up label
  cardinality (`prometheus_client` metric families do not evict entries). This
  is within the operator-trusted-MCP threat model, but if you expose the gateway
  publicly, restrict `/metrics` at your reverse proxy the same way you would any
  operational endpoint, and keep MCP servers trusted.

### Metrics emitted

| Metric | Type | Labels | Source |
|---|---|---|---|
| `thinclaw_llm_requests_total` | counter | provider, model | each LLM request |
| `thinclaw_llm_errors_total` | counter | provider, model | failed LLM responses |
| `thinclaw_llm_response_seconds` | histogram | provider, model | LLM response latency |
| `thinclaw_tool_calls_total` | counter | tool, success | each tool invocation |
| `thinclaw_tool_call_seconds` | histogram | tool | tool execution latency |
| `thinclaw_agent_turns_total` | counter | — | completed reasoning turns |
| `thinclaw_agent_errors_total` | counter | component | component errors |
| `thinclaw_channel_messages_total` | counter | channel, direction | channel traffic |
| `thinclaw_heartbeat_ticks_total` | counter | — | heartbeat ticks |
| `thinclaw_tokens_used_total` | counter | — | cumulative model tokens |
| `thinclaw_request_latency_seconds` | histogram | — | generic request latency |
| `thinclaw_active_jobs` | gauge | — | active jobs |
| `thinclaw_queue_depth` | gauge | — | message queue depth |
| `thinclaw_cost_cents` | gauge | — | cumulative spend (refreshed from `CostTracker` at scrape time) |

### OTLP / OpenTelemetry export

Push export over OTLP is **not** wired yet. Its tonic-based gRPC exporter would
conflict with the tonic version libSQL already pins for remote replicas, so it
is deferred to a follow-up behind its own opt-in feature flag. The pull-based
Prometheus endpoint above covers the common scrape-and-dashboard workflow.

## The `/api/health` readiness probe

`GET /api/health` returns `200 healthy` only when **all** of the following hold,
otherwise `503 unhealthy` (so load balancers route away from a broken instance):

1. the database is reachable within 2s (an instance with no persistence
   configured is treated as DB-ready),
2. at least one LLM provider is configured, and
3. the gateway's inbound message channel is wired to the agent runtime.

The decision is a pure function (`thinclaw_gateway::web::status::readiness_response`)
and is unit-tested independently of the runtime.
