#!/usr/bin/env python3
"""
MLX STT Server — OpenAI-compatible /v1/audio/transcriptions endpoint.

Wraps `mlx-whisper` to serve speech-to-text over HTTP, matching
the OpenAI Whisper API surface.

Usage:
    python mlx_stt_server.py --model <hf-repo-or-path> --port 53757 [--host 127.0.0.1]
"""

import argparse
import json
import os
import sys
import tempfile
import time
from http.server import HTTPServer, BaseHTTPRequestHandler
from io import BytesIO

import mlx_whisper


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
                kwargs = {"path_or_hf_repo": _model_path}
                if language:
                    kwargs["language"] = language

                result = mlx_whisper.transcribe(tmp_path, **kwargs)
                text = result.get("text", "")

                # Return in OpenAI format
                response_format = "json"  # default
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

    # Warm up: do a small transcription to load the model into memory
    print(f"[mlx-stt] Pre-loading model: {_model_path}", flush=True)
    # mlx_whisper lazily loads on first transcribe, so the server starts fast

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
