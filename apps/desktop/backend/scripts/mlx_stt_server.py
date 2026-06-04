#!/usr/bin/env python3
"""
MLX STT Server — OpenAI-compatible /v1/audio/transcriptions endpoint.

Wraps `mlx-whisper` to serve speech-to-text over HTTP, matching
the OpenAI Whisper API surface.

Usage:
    python mlx_stt_server.py --model <hf-repo-or-path> --port 53757 [--host 127.0.0.1]
"""

import argparse
import dataclasses
import json
import os
import sys
import tempfile
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from io import BytesIO
from pathlib import Path

# ---------------------------------------------------------------------------
# Fix mlx_whisper compatibility with mlx-community 4-bit models
# ---------------------------------------------------------------------------
#
# mlx_whisper 0.4.x load_models.py only looks for:
#   - weights.safetensors
#   - weights.npz
# But mlx-community 4-bit models (e.g. whisper-small-4bit) name the file
# model.safetensors.  We create a weights.safetensors symlink at startup so
# the original mlx_whisper code finds it without any monkey-patching.
#
# Why not monkey-patch?  transcribe.py does:
#   from .load_models import load_model   ← direct binding, not a module ref
# so patching the module attribute has no effect on the already-bound name.
#
# Also fix ModelDimensions(**config): newer model configs may include extra
# keys (e.g. sample_rate) not accepted by ModelDimensions.  We patch the
# load_models module to strip them before constructing ModelDimensions.

try:
    import dataclasses as _dc
    import mlx_whisper.load_models as _lm
    import mlx_whisper.whisper as _mw
    import mlx.core as _mx_core
    import mlx.nn as _mx_nn
    from mlx.utils import tree_unflatten as _tree_unflatten
    from pathlib import Path as _LmPath
    import json as _lm_json

    def _safe_load_model(path_or_hf_repo, dtype=None):
        """
        Patched version of load_model that:
        1. Strips unknown keys from config.json (e.g. sample_rate = not in ModelDimensions)
        2. Supports model.safetensors in addition to weights.safetensors / weights.npz
        """
        model_path = _LmPath(path_or_hf_repo)
        if not model_path.exists():
            from huggingface_hub import snapshot_download
            model_path = _LmPath(snapshot_download(repo_id=str(path_or_hf_repo)))

        with open(str(model_path / "config.json")) as f:
            config = _lm_json.load(f)

        config.pop("model_type", None)
        quantization = config.pop("quantization", None)

        known = {field.name for field in _dc.fields(_mw.ModelDimensions)}
        for k in [k for k in list(config) if k not in known]:
            print(f"[mlx-stt] Ignoring unknown config key: {k}", flush=True)
            config.pop(k)

        model_args = _mw.ModelDimensions(**config)

        wf = model_path / "weights.safetensors"
        if not wf.exists():
            wf = model_path / "weights.npz"
        if not wf.exists():
            wf = model_path / "model.safetensors"

        print(f"[mlx-stt] Loading weights: {wf.name}", flush=True)
        weights = _mx_core.load(str(wf))

        if dtype is None:
            dtype = _mx_core.float16
        model = _mw.Whisper(model_args, dtype)

        if quantization is not None:
            nn_quantize = _mx_nn.quantize
            nn_quantize(
                model,
                **quantization,
                class_predicate=lambda p, m: isinstance(m, (_mx_nn.Linear, _mx_nn.Embedding))
                    and f"{p}.scales" in weights
            )

        weights = _tree_unflatten(list(weights.items()))
        model.update(weights)
        _mx_core.eval(model.parameters())
        return model

    # Patch at the module level AND at the call site (transcribe.py binds load_model directly)
    _lm.load_model = _safe_load_model
    import mlx_whisper.transcribe as _tr_mod
    # transcribe.ModelHolder.get_model uses load_model from its __globals__
    # which IS transcribe.__dict__ — so this assignment IS seen at call time
    _tr_mod.__dict__['load_model'] = _safe_load_model
    print("[mlx-stt] Compatibility patches applied (ModelDimensions + model.safetensors).", flush=True)
except Exception as _pe:
    print(f"[mlx-stt] Warning: patch failed ({_pe}). Will attempt symlink fallback.", flush=True)

import mlx_whisper


def _prepare_model_weights(model_path: str):
    """
    Ensure mlx_whisper can find the model weights:
    - If model.safetensors exists but weights.safetensors doesn't,
      create a weights.safetensors symlink (symlink is the belt-and-suspenders
      fallback in case the module-level patch doesn't intercept the call).
    """
    p = Path(model_path)
    if not p.is_dir():
        return
    model_st = p / "model.safetensors"
    weights_st = p / "weights.safetensors"
    if model_st.exists() and not weights_st.exists():
        try:
            weights_st.symlink_to("model.safetensors")
            print(f"[mlx-stt] Created symlink: weights.safetensors -> model.safetensors", flush=True)
        except Exception as e:
            print(f"[mlx-stt] Warning: could not create symlink: {e}", flush=True)



# ---------------------------------------------------------------------------
# Model validation
# ---------------------------------------------------------------------------
def validate_whisper_model(model_path: str) -> str:
    """
    Validate that model_path points to a Whisper-architecture MLX model.
    Returns an error string if invalid, or empty string if OK.
    """
    p = Path(model_path)

    # Check for HuggingFace-format directory
    if p.is_dir():
        config_file = p / "config.json"
        if not config_file.exists():
            return (
                f"No config.json found in {model_path}. "
                "This does not appear to be a valid MLX model directory. "
                "Please download a Whisper model via the Discover tab (e.g. mlx-community/whisper-large-v3-turbo)."
            )

        try:
            with open(config_file) as f:
                cfg = json.load(f)
        except Exception as e:
            return f"Failed to read config.json: {e}"

        model_type = cfg.get("model_type", "")
        # Whisper models have model_type "whisper" or no model_type (older MLX exports)
        # Parakeet / NeMo models have no model_type but have NeMo-specific keys
        nemo_keys = {"preprocessor", "encoder", "decoder", "joint", "decoding", "rnnt_reduction"}
        if nemo_keys.intersection(cfg.keys()):
            return (
                f"Model at {model_path} appears to be a NeMo/Parakeet model, "
                "which is not supported by mlx-whisper. "
                "Please use a Whisper-architecture model such as "
                "mlx-community/whisper-large-v3-turbo or mlx-community/whisper-small."
            )

        if model_type and model_type != "whisper":
            return (
                f"Model type '{model_type}' is not supported by mlx-whisper. "
                "Please use a Whisper-architecture model."
            )

        # Check for weights file — mlx-community uses different naming conventions:
        #   weights.safetensors (older exports), model.safetensors (4-bit), weights.npz (legacy)
        has_weights = (
            (p / "weights.safetensors").exists() or
            (p / "weights.npz").exists() or
            (p / "model.safetensors").exists()
        )
        if not has_weights:
            return (
                f"No weights.safetensors, model.safetensors, or weights.npz found in {model_path}. "
                "The model may be incomplete — try re-downloading it."
            )

    elif p.suffix in (".bin", ".gguf", ".ggml"):
        return (
            f"Model file {p.name} is a GGML/GGUF binary — only compatible with whisper.cpp. "
            "For the MLX engine, please download an MLX Whisper model via the Discover tab "
            "(e.g. mlx-community/whisper-large-v3-turbo)."
        )
    else:
        return (
            f"Unrecognized model format at {model_path}. "
            "Please use an MLX Whisper model directory from HuggingFace."
        )

    return ""


# ---------------------------------------------------------------------------
# Global state
# ---------------------------------------------------------------------------
_model_path = ""


# ---------------------------------------------------------------------------
# HTTP Handler — OpenAI /v1/audio/transcriptions compatible
# ---------------------------------------------------------------------------
class STTHandler(BaseHTTPRequestHandler):
    api_key = None

    def log_message(self, format, *args):
        print(f"[mlx-stt] {args[0]}", flush=True)

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
            self._send_json({"status": "ok", "model": _model_path})
        else:
            self._send_json({"error": "Not found"}, 404)

    def do_POST(self):
        if not self._check_auth():
            self._send_json({"error": "Unauthorized"}, 401)
            return

        if self.path != "/v1/audio/transcriptions":
            self._send_json({"error": "Not found"}, 404)
            return

        try:
            content_type = self.headers.get("Content-Type", "")
            content_length = int(self.headers.get("Content-Length", 0))
            raw_body = self.rfile.read(content_length)

            # Handle multipart/form-data (standard OpenAI whisper API format)
            if "multipart/form-data" in content_type:
                audio_data, language = self._parse_multipart(raw_body, content_type)
            else:
                # Raw audio bytes
                audio_data = raw_body
                language = None

            # Write to temp file for mlx_whisper
            suffix = ".wav"
            with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as f:
                f.write(audio_data)
                tmp_path = f.name

            try:
                # mlx_whisper.transcribe does not have a 'language' parameter in 0.4.x
                result = mlx_whisper.transcribe(tmp_path, path_or_hf_repo=_model_path)
                text = result.get("text", "")

                # Return in OpenAI format
                self._send_json({"text": text})
            finally:
                os.unlink(tmp_path)

        except Exception as e:
            print(f"[mlx-stt] Error: {e}", flush=True)
            import traceback
            traceback.print_exc()
            self._send_json({"error": str(e)}, 500)

    def _parse_multipart(self, body: bytes, content_type: str):
        """Minimal multipart parser to extract the 'file' field."""
        # Extract boundary
        boundary = None
        for part in content_type.split(";"):
            part = part.strip()
            if part.startswith("boundary="):
                boundary = part[len("boundary="):].strip('"')
                break

        if not boundary:
            return body, None

        boundary_bytes = f"--{boundary}".encode()
        parts = body.split(boundary_bytes)
        audio_data = None
        language = None

        for part in parts:
            if b"Content-Disposition" not in part:
                continue

            header_end = part.find(b"\r\n\r\n")
            if header_end == -1:
                continue

            headers = part[:header_end].decode("utf-8", errors="replace")
            data = part[header_end + 4:]
            # Strip trailing \r\n-- or \r\n
            if data.endswith(b"\r\n"):
                data = data[:-2]
            if data.endswith(b"--"):
                data = data[:-2]
            if data.endswith(b"\r\n"):
                data = data[:-2]

            if 'name="file"' in headers:
                audio_data = data
            elif 'name="language"' in headers:
                language = data.decode("utf-8").strip()

        return audio_data or body, language


def main():
    parser = argparse.ArgumentParser(description="MLX STT Server")
    parser.add_argument("--model", required=True, help="HF repo ID or local path (e.g. mlx-community/whisper-large-v3-turbo)")
    parser.add_argument("--port", type=int, default=53757, help="Server port")
    parser.add_argument("--host", default="127.0.0.1", help="Server host")
    parser.add_argument("--api-key", default=None, help="Optional API key")
    args = parser.parse_args()

    global _model_path
    _model_path = args.model

    # Validate model before starting server
    err = validate_whisper_model(_model_path)
    if err:
        print(f"[mlx-stt] ERROR: {err}", flush=True)
        sys.exit(1)

    # Prepare model weights (symlink model.safetensors → weights.safetensors if needed)
    _prepare_model_weights(_model_path)

    print(f"[mlx-stt] Model validated: {_model_path}", flush=True)

    STTHandler.api_key = args.api_key
    server = HTTPServer((args.host, args.port), STTHandler)
    print(f"[mlx-stt] Server listening on {args.host}:{args.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("[mlx-stt] Shutting down", flush=True)
        server.server_close()


if __name__ == "__main__":
    main()
