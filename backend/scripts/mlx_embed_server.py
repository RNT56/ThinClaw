#!/usr/bin/env python3
"""
MLX Embedding Server — OpenAI-compatible /v1/embeddings endpoint.

Wraps `mlx-embeddings` to serve embedding vectors over HTTP, matching
the same API surface as llama-server --embedding or OpenAI's API.

Usage:
    python mlx_embed_server.py --model <hf-repo-or-path> --port 53756 [--host 127.0.0.1]
"""

import argparse
import json
import sys
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from pathlib import Path
from typing import List

import mlx.core as mx
from mlx_embeddings.utils import load


# ---------------------------------------------------------------------------
# Global model state (loaded once at startup)
# ---------------------------------------------------------------------------
_model = None
_tokenizer = None
_model_name = ""


def validate_embedding_model(model_path: str) -> str:
    """
    Validate that model_path refers to an MLX-compatible embedding model.
    Returns a human-readable error string if invalid, or "" if OK.
    """
    p = Path(model_path)

    # GGUF / GGML / binary files — only work with llama.cpp
    if p.is_file():
        if p.suffix in (".gguf", ".ggml", ".bin", ".pt", ".safetensors"):
            return (
                f"Model file '{p.name}' is a single binary file. "
                "MLX embedding requires a HuggingFace model directory with config.json and safetensors weights. "
                "Please download an MLX-compatible embedding model via the Discover tab, "
                "e.g. 'mlx-community/mxbai-embed-xsmall-v1' or 'mlx-community/bge-small-en-v1.5-4bit'."
            )
        return f"Unrecognized model format: {model_path}"

    if p.is_dir():
        config_file = p / "config.json"
        if not config_file.exists():
            # Check if it contains only binary files (GGUF downloaded into a folder)
            files = list(p.iterdir())
            binary_exts = {".gguf", ".ggml", ".bin"}
            if files and all(f.suffix in binary_exts for f in files if f.is_file()):
                return (
                    f"Directory '{p.name}' contains only GGUF/binary files which require llama.cpp. "
                    "Please download an MLX-native model from the Discover tab, "
                    "e.g. 'mlx-community/mxbai-embed-xsmall-v1'."
                )
            return (
                f"No config.json found in '{p.name}'. "
                "This doesn't appear to be a valid HuggingFace model directory. "
                "Download an MLX embedding model via the Discover tab."
            )
        return ""

    return f"Model path not found: {model_path}"


def load_model(model_name: str):
    global _model, _tokenizer, _model_name
    print(f"[mlx-embed] Loading model: {model_name}", flush=True)
    _model, _tokenizer = load(model_name)
    _model_name = model_name
    print(f"[mlx-embed] Model loaded successfully", flush=True)


def embed_texts(texts: List[str]) -> List[List[float]]:
    """Generate embeddings for a list of texts."""
    # Some tokenizers (e.g. GemmaTokenizer) are wrapped by mlx_embeddings in a
    # TokenizerWrapper that doesn't expose batch_encode_plus.
    # Fall back to direct __call__ which all tokenizer types support.
    encode_kwargs = dict(
        return_tensors="mlx",
        padding=True,
        truncation=True,
        max_length=512,
    )

    if hasattr(_tokenizer, "batch_encode_plus"):
        inputs = _tokenizer.batch_encode_plus(texts, **encode_kwargs)
    else:
        # TokenizerWrapper / GemmaTokenizer: use __call__ directly
        inputs = _tokenizer(texts, **encode_kwargs)

    input_ids = inputs["input_ids"]
    attention_mask = inputs.get("attention_mask")

    if attention_mask is not None:
        outputs = _model(input_ids, attention_mask=attention_mask)
    else:
        outputs = _model(input_ids)


    # Use mean-pooled normalized embeddings if available, else CLS token
    if hasattr(outputs, "text_embeds") and outputs.text_embeds is not None:
        emb = outputs.text_embeds
    else:
        emb = outputs.last_hidden_state[:, 0, :]

    # Convert to list of lists
    results = []
    if len(emb.shape) == 1:
        # Single embedding (shouldn't happen with batch, but be safe)
        results.append(emb.tolist())
    else:
        for i in range(emb.shape[0]):
            results.append(emb[i].tolist())

    return results


# ---------------------------------------------------------------------------
# HTTP Handler — OpenAI /v1/embeddings compatible
# ---------------------------------------------------------------------------
class EmbeddingHandler(BaseHTTPRequestHandler):
    api_key = None

    def log_message(self, format, *args):
        print(f"[mlx-embed] {args[0]}", flush=True)

    def _check_auth(self) -> bool:
        if not self.api_key:
            return True
        auth = self.headers.get("Authorization", "")
        return auth == f"Bearer {self.api_key}"

    def _send_json(self, data: dict, status: int = 200):
        body = json.dumps(data).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health" or self.path == "/":
            self._send_json({"status": "ok", "model": _model_name})
        else:
            self._send_json({"error": "Not found"}, 404)

    def do_POST(self):
        if not self._check_auth():
            self._send_json({"error": "Unauthorized"}, 401)
            return

        if self.path != "/v1/embeddings":
            self._send_json({"error": "Not found"}, 404)
            return

        try:
            content_length = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(content_length))

            input_data = body.get("input", "")
            if isinstance(input_data, str):
                texts = [input_data]
            elif isinstance(input_data, list):
                texts = [str(t) for t in input_data]
            else:
                self._send_json({"error": "Invalid input"}, 400)
                return

            embeddings = embed_texts(texts)

            response = {
                "object": "list",
                "data": [
                    {
                        "object": "embedding",
                        "embedding": emb,
                        "index": i,
                    }
                    for i, emb in enumerate(embeddings)
                ],
                "model": body.get("model", _model_name),
                "usage": {
                    "prompt_tokens": sum(len(t.split()) for t in texts),
                    "total_tokens": sum(len(t.split()) for t in texts),
                },
            }
            self._send_json(response)

        except Exception as e:
            print(f"[mlx-embed] Error: {e}", flush=True)
            self._send_json({"error": str(e)}, 500)


def main():
    parser = argparse.ArgumentParser(description="MLX Embedding Server")
    parser.add_argument("--model", required=True, help="HF repo ID or local path")
    parser.add_argument("--port", type=int, default=53756, help="Server port")
    parser.add_argument("--host", default="127.0.0.1", help="Server host")
    parser.add_argument("--api-key", default=None, help="Optional API key")
    args = parser.parse_args()

    # Validate model compatibility before trying to load it
    err = validate_embedding_model(args.model)
    if err:
        print(f"[mlx-embed] ERROR: {err}", flush=True)
        sys.exit(1)

    load_model(args.model)

    EmbeddingHandler.api_key = args.api_key
    server = HTTPServer((args.host, args.port), EmbeddingHandler)
    print(f"[mlx-embed] Server listening on {args.host}:{args.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("[mlx-embed] Shutting down", flush=True)
        server.server_close()


if __name__ == "__main__":
    main()
