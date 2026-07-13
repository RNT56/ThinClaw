# RAG And Embedding Compatibility

Last verified: 2026-07-13

This document is the compatibility and failure-mode contract for Direct
Workbench retrieval. The vector index is disposable acceleration state; the
SQLite document/chunk rows and full-text index remain the durable fallback.

## Current provider defaults

| Provider | Default model | Stored dimensions | Retrieval behavior |
|---|---|---:|---|
| Google Gemini | `gemini-embedding-2` | 768 | Uses Google's documented query/document instruction formats and explicitly requests 768 dimensions. |
| Voyage AI | `voyage-4` | 1024 | Uses `document` for ingestion and `query` for retrieval. |
| Cohere | `embed-multilingual-v3.0` | 1024 | Uses `search_document` for ingestion and `search_query` for retrieval. |
| OpenAI | `text-embedding-3-small` | 1536 | Requests the declared dimension explicitly. |
| Local sidecar | selected model | live-detected | The running OpenAI-compatible endpoint is probed before an index is opened. |

Persisted Gemini selections for `text-embedding-004`, `embedding-001`, or
`gemini-embedding-001` migrate to Embedding 2 so upgrades do not retain a model
at or beyond its provider shutdown date.

The Gemini and Voyage selections follow their current first-party model
catalogs. Gemini Embedding 2 supports the batch endpoint and recommends 768,
1536, or 3072 dimensions; Voyage 4 defaults to 1024 dimensions. Sources:
[Google embeddings](https://ai.google.dev/gemini-api/docs/embeddings),
[Gemini deprecations](https://ai.google.dev/gemini-api/docs/deprecations), and
[Voyage text embeddings](https://docs.voyageai.com/docs/embeddings). The static
Voyage catalog pricing is sourced from the current
[Voyage pricing table](https://docs.voyageai.com/docs/pricing).

Every provider response is rejected unless its vector count, dimension, and
finite-number invariants match the request and backend declaration. A failed
cloud request may use an available local sidecar; a successful cloud request
with zero matches does not mix embedding spaces by falling through to local.

## Dimension authority and migration

Dimension discovery is ordered as follows:

1. Cloud backends declare and, where supported, request a fixed output size.
2. Hugging Face `config.json` is a bounded hint. Sentence/projection-specific
   fields take priority over transformer hidden size, including nested text
   configs. Zero and values above 65,536 are rejected.
3. A local server's actual `/v1/embeddings` response is authoritative. ThinClaw
   waits for readiness, validates the probe, then opens the scoped vector index.

When a dimension changes, ThinClaw persists the new size first, clears cached
stores, and deletes only incompatible `.usearch` files. Documents and FTS rows
are retained, so keyword retrieval remains available. The UI receives an
`embedding_dims_changed` event explaining that documents should be re-imported
to rebuild semantic search.

Configured cloud backends are restored during application startup. Unknown
providers or selections without a usable key clear the previous backend rather
than leaving stale credentials/routing active.

## Reranker artifact contract

The lightweight ONNX cross-encoder remains
[`Xenova/ms-marco-MiniLM-L-6-v2`](https://huggingface.co/Xenova/ms-marco-MiniLM-L-6-v2)
to keep the automatic desktop download bounded. Both files are pinned to
revision `a09144355adeed5f58c8ed011d209bf8ee5a1fec` and verified before load:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `onnx/model_quantized.onnx` | 23,143,499 | `e9d8ebf845c413e981c175bfe49a3bfa9b3dcce2a3ba54875ee5df5a58639fbe` |
| `tokenizer.json` | 711,396 | `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` |

Downloads use status checks, timeouts, a strict byte ceiling, three bounded
attempts, streaming SHA-256 verification, and same-directory staging before
replacement. Inference is capped at 150 candidates and 512 tokens per pair.
Initialization or runtime failure preserves reciprocal-rank-fusion order; it
does not fail the whole RAG answer.

## Acceptance

```bash
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 \
  cargo check --manifest-path apps/desktop/backend/Cargo.toml --locked

CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --manifest-path apps/desktop/backend/Cargo.toml --locked \
  embedding_dimension --lib

CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --manifest-path apps/desktop/backend/Cargo.toml --locked \
  reranker::tests --lib
```

Real-provider smoke remains credential-gated; it must not be represented as
completed by the deterministic unit/compile acceptance above.
