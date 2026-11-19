#!/usr/bin/env python3
"""Private JSON-lines worker for the pinned local Laya MLX checkpoint."""

from __future__ import annotations

import importlib.metadata
import hashlib
import json
import math
import os
import selectors
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

MODEL_REVISION = "ba40c87fcb357f1643d04d71323af9cdc3b9e591"
MODEL_REPOSITORY = "aac6fef/laya-multilingual-mlx"
RUNTIME_VERSION = "0.3.0"
MLX_VERSION = "0.32.3"
MODEL_SHA256 = "7fc5834af4d8fdfb268d272a9d1a66e5819a0daac98241651c4c888cc43adff1"
MAX_REQUEST_BYTES = 32_768
MAX_STATE_BYTES = 24_576
MAX_QUESTION_BYTES = 2_048
MAX_OPTIONS_BYTES = 2_048
MAX_LABEL_BYTES = 128
MAX_OPTIONS = 8


def fail(message: str) -> dict[str, Any]:
    return {"ok": False, "status": "error", "diagnostics": [message]}


def validate(request: Any) -> tuple[Any, str, list[str]]:
    if not isinstance(request, dict) or set(request) != {"id", "state", "question", "options"}:
        raise ValueError("request must contain exactly id, state, question, and options")
    if not isinstance(request["id"], (int, str)) or isinstance(request["id"], bool):
        raise ValueError("id must be a string or integer")
    state = request["state"]
    if state is None or not isinstance(state, (str, dict, list)):
        raise ValueError("state must be text, an object, or an array")
    state_bytes = json.dumps(state, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(state_bytes) > MAX_STATE_BYTES:
        raise ValueError(f"state exceeds {MAX_STATE_BYTES} UTF-8 bytes")
    question = request["question"]
    if not isinstance(question, str) or not question.strip():
        raise ValueError("question must be nonempty text")
    if len(question.encode("utf-8")) > MAX_QUESTION_BYTES:
        raise ValueError(f"question exceeds {MAX_QUESTION_BYTES} UTF-8 bytes")
    options = request["options"]
    if not isinstance(options, list) or not 2 <= len(options) <= MAX_OPTIONS:
        raise ValueError(f"options must contain 2 to {MAX_OPTIONS} labels")
    if any(not isinstance(label, str) or not label.strip() for label in options):
        raise ValueError("every option must be nonempty text")
    if len(set(options)) != len(options):
        raise ValueError("option labels must be unique")
    if any(len(label.encode("utf-8")) > MAX_LABEL_BYTES for label in options):
        raise ValueError(f"each option must be at most {MAX_LABEL_BYTES} UTF-8 bytes")
    if sum(len(label.encode("utf-8")) for label in options) > MAX_OPTIONS_BYTES:
        raise ValueError(f"options exceed {MAX_OPTIONS_BYTES} UTF-8 bytes in total")
    return state, question, options


def load_agent() -> tuple[Any, dict[str, Any]]:
    model_path = Path(os.environ.get("LAYA_MODEL_PATH", "")).expanduser().resolve()
    if not model_path.is_dir() or model_path.name != MODEL_REVISION:
        raise RuntimeError(
            "LAYA_MODEL_PATH must point to the cached snapshot directory for the pinned revision "
            f"{MODEL_REVISION}"
        )
    if not (model_path / "model.safetensors").is_file():
        raise RuntimeError("pinned model snapshot is incomplete: model.safetensors is missing")

    digest = hashlib.sha256()
    with (model_path / "model.safetensors").open("rb") as checkpoint:
        for chunk in iter(lambda: checkpoint.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != MODEL_SHA256:
        raise RuntimeError("pinned model checkpoint checksum does not match")

    runtime_version = importlib.metadata.version("laya-mlx")
    mlx_version = importlib.metadata.version("mlx")
    if runtime_version != RUNTIME_VERSION or mlx_version != MLX_VERSION:
        raise RuntimeError(
            f"expected laya-mlx {RUNTIME_VERSION} and mlx {MLX_VERSION}; "
            f"found laya-mlx {runtime_version} and mlx {mlx_version}"
        )

    import laya_mlx as laya

    started = time.perf_counter()
    agent = laya.load(str(model_path), dtype="float16")
    model_load_ms = (time.perf_counter() - started) * 1000
    return agent, {
        "model_repository": MODEL_REPOSITORY,
        "model_revision": MODEL_REVISION,
        "runtime_version": runtime_version,
        "mlx_version": mlx_version,
        "model_load_ms": round(model_load_ms, 3),
    }


def predict(agent: Any, metadata: dict[str, Any], request: Any) -> dict[str, Any]:
    request_id = request.get("id") if isinstance(request, dict) else None
    try:
        state, question, options = validate(request)
        started = time.perf_counter()
        result = agent.predict(
            state,
            {
                "decision": {
                    "type": "choice",
                    "instructions": question,
                    "criteria": options,
                }
            },
        )
        elapsed_ms = (time.perf_counter() - started) * 1000
        usage = result.get("usage", {})
        answer = result.get("answers", {}).get("decision", {})
        diagnostics = []
        input_tokens = usage.get("input_tokens")
        dropped_tokens = usage.get("state_tokens_dropped", 0)
        if not isinstance(input_tokens, int) or input_tokens < 0:
            return {"id": request_id, **fail("model returned invalid input token usage")}
        if not isinstance(dropped_tokens, int) or dropped_tokens < 0:
            return {"id": request_id, **fail("model returned invalid dropped token usage")}
        truncated = bool(usage.get("truncated")) or dropped_tokens > 0
        if truncated:
            diagnostics.append("model input was truncated")
        if usage.get("options"):
            diagnostics.append("candidate option token spans collided")
        probabilities = answer.get("probabilities")
        if not isinstance(probabilities, dict) or set(probabilities) != set(options):
            diagnostics.append("model returned an incomplete probability set")
        if diagnostics:
            return {
                "id": request_id,
                "ok": True,
                "status": "abstain",
                "diagnostics": diagnostics,
                "truncated": truncated,
                "input_tokens": input_tokens,
                "dropped_tokens": dropped_tokens,
                **metadata,
        "inference_ms": round(elapsed_ms, 3),
            }
        values = [float(probabilities[label]) for label in options]
        if any(not math.isfinite(value) or not 0 <= value <= 1 for value in values):
            return {"id": request_id, **fail("model returned a non-finite or invalid probability")}
        if not math.isclose(sum(values), 1.0, abs_tol=0.002):
            return {"id": request_id, **fail("model probabilities do not sum to one")}
        confidence = float(answer["answer_confidence"])
        if not math.isfinite(confidence) or not 0 <= confidence <= 1:
            return {"id": request_id, **fail("model returned an invalid answer confidence")}
        ordered = sorted(values, reverse=True)
        return {
            "id": request_id,
            "ok": True,
            "status": "prediction",
            "probabilities": probabilities,
            "answer_confidence": confidence,
            "margin": ordered[0] - ordered[1],
            "truncated": False,
            "diagnostics": [],
            **metadata,
            "inference_ms": round(elapsed_ms, 3),
            "input_tokens": input_tokens,
            "dropped_tokens": dropped_tokens,
        }
    except (ValueError, TypeError, KeyError, UnicodeError) as error:
        return {"id": request_id, **fail(str(error))}


def emit(value: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def reject_constant(value: str) -> None:
    raise ValueError(f"invalid JSON number: {value}")


def serve() -> int:
    try:
        agent, metadata = load_agent()
    except Exception as error:
        print(f"Laya worker unavailable: {error}", file=sys.stderr)
        return 2
    emit({"type": "ready", **metadata})
    while True:
        raw_line = sys.stdin.buffer.readline(MAX_REQUEST_BYTES + 1)
        if not raw_line:
            break
        if len(raw_line) > MAX_REQUEST_BYTES:
            while raw_line and not raw_line.endswith(b"\n"):
                raw_line = sys.stdin.buffer.readline(4096)
            emit(fail(f"request exceeds {MAX_REQUEST_BYTES} UTF-8 bytes"))
            continue
        try:
            request = json.loads(
                raw_line.decode("utf-8"),
                parse_constant=reject_constant,
            )
        except (json.JSONDecodeError, UnicodeDecodeError, ValueError) as error:
            emit(fail(f"invalid JSON request: {error}"))
            continue
        emit(predict(agent, metadata, request))
    return 0


def read_line(process: subprocess.Popen[bytes], pending: bytearray, seconds: float) -> bytes:
    deadline = time.monotonic() + seconds
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        while b"\n" not in pending:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                raise TimeoutError("worker did not reply before the check deadline")
            chunk = os.read(process.stdout.fileno(), 4096)
            if not chunk:
                raise RuntimeError(f"worker exited with status {process.poll()}")
            pending.extend(chunk)
    line, _, rest = pending.partition(b"\n")
    pending[:] = rest
    return line


def run_check() -> int:
    model_path = os.environ.get("LAYA_MODEL_PATH")
    if not model_path:
        raise RuntimeError("set LAYA_MODEL_PATH to the downloaded pinned snapshot before running --check")
    env = {key: os.environ[key] for key in ("PATH", "HOME", "LAYA_MODEL_PATH") if key in os.environ}
    env.update(
        HF_HUB_OFFLINE="1",
        TRANSFORMERS_OFFLINE="1",
        HF_HUB_DISABLE_TELEMETRY="1",
    )
    cases = [
        {
            "id": "english",
            "state": "The customer was billed twice and asks us to refund the duplicate payment.",
            "question": "Which team should handle this request?",
            "options": ["billing", "technical support", "sales"],
        },
        {
            "id": "lithuanian",
            "state": "Klientui du kartus nuskaičiuota suma. Jis prašo grąžinti pinigus.",
            "question": "Kuris skyrius turėtų nagrinėti šį prašymą?",
            "options": ["sąskaitos ir grąžinimai", "techninė pagalba", "pardavimai"],
        },
        {
            "id": "elementor_mapping_en",
            "state": {"css": "padding: 16px", "target": "Elementor section"},
            "question": "Which destination best matches this CSS? Treat native Elementor style versus custom CSS as separate choices.",
            "options": ["native Elementor style", "custom CSS", "insufficient evidence"],
        },
        {
            "id": "elementor_mapping_lt",
            "state": {"css": "padding: 16px", "target": "Elementor section"},
            "question": "Kur geriausia perkelti šią CSS taisyklę? Atskirai įvertink Elementor stiliaus nustatymą ir pasirinktinį CSS.",
            "options": ["Elementor stiliaus nustatymas", "pasirinktinis CSS", "nepakanka duomenų"],
        },
        {
            "id": "elementor_pattern_en",
            "state": "Several sections repeat the same heading, image, and button layout across the page.",
            "question": "Which pattern category best describes this repeated structure?",
            "options": ["reusable component", "one-off section", "global color"],
        },
        {
            "id": "elementor_pattern_lt",
            "state": "Keliuose puslapio skyriuose kartojasi ta pati antraštės, paveikslėlio ir mygtuko struktūra.",
            "question": "Kuri kategorija geriausiai apibūdina pasikartojančią struktūrą?",
            "options": ["pakartotinai naudojamas komponentas", "vienkartinis skyrius", "globali spalva"],
        },
    ]

    process = subprocess.Popen(
        [sys.executable, str(Path(__file__).resolve())],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=None,
        env=env,
        bufsize=0,
    )
    pending = bytearray()
    try:
        ready = json.loads(read_line(process, pending, 180))
        assert ready["type"] == "ready"
        assert ready["model_revision"] == MODEL_REVISION
        assert ready["runtime_version"] == "0.3.0"
        for case in cases:
            process.stdin.write(json.dumps(case, ensure_ascii=False).encode("utf-8") + b"\n")
            response = json.loads(read_line(process, pending, 60))
            assert response["id"] == case["id"]
            assert response["ok"] is True and response["status"] == "prediction", response
            assert response["model_revision"] == MODEL_REVISION
            assert response["runtime_version"] == "0.3.0"
            assert response["truncated"] is False
            assert response["input_tokens"] > 0
            assert response["dropped_tokens"] == 0
            assert all(math.isfinite(value) and 0 <= value <= 1 for value in response["probabilities"].values())
            assert set(response["probabilities"]) == set(case["options"])
            assert math.isclose(sum(response["probabilities"].values()), 1, abs_tol=0.002)
            assert math.isfinite(response["answer_confidence"])
            assert math.isfinite(response["margin"])
            print(json.dumps({"case": case["id"], **response}, ensure_ascii=False))

        long_state = "padding: 16px; " * 1_500
        assert len(json.dumps(long_state, ensure_ascii=False).encode("utf-8")) <= MAX_STATE_BYTES
        truncated = {
            "id": "actual_truncation",
            "state": long_state,
            "question": "Map this CSS to the best Elementor destination.",
            "options": ["native Elementor style", "custom CSS", "insufficient evidence"],
        }
        process.stdin.write(json.dumps(truncated).encode("utf-8") + b"\n")
        truncated_response = json.loads(read_line(process, pending, 60))
        assert truncated_response["ok"] is True and truncated_response["status"] == "abstain", truncated_response
        assert truncated_response["truncated"] is True, truncated_response
        assert truncated_response["input_tokens"] > 0, truncated_response
        assert truncated_response["dropped_tokens"] > 0, truncated_response
        assert "model input was truncated" in truncated_response["diagnostics"], truncated_response
        assert "probabilities" not in truncated_response, truncated_response
        print(json.dumps({"case": "actual_truncation", **truncated_response}, ensure_ascii=False))

        process.stdin.write(b"\xff\n")
        invalid_utf8 = json.loads(read_line(process, pending, 5))
        assert invalid_utf8["ok"] is False
        too_long = {**cases[0], "question": "ą" * 1_025}
        process.stdin.write(json.dumps(too_long, ensure_ascii=False).encode("utf-8") + b"\n")
        oversized = json.loads(read_line(process, pending, 5))
        assert oversized["ok"] is False
        assert "UTF-8 bytes" in oversized["diagnostics"][0]
        invalid = {**cases[0], "options": ["same", "same"]}
        process.stdin.write(json.dumps(invalid).encode("utf-8") + b"\n")
        malformed = json.loads(read_line(process, pending, 5))
        assert malformed["ok"] is False
        assert "unique" in malformed["diagnostics"][0]
        process.terminate()
        process.wait(timeout=10)
        assert process.returncode is not None
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()

    timed = subprocess.Popen(
        [sys.executable, str(Path(__file__).resolve())],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=env,
    )
    try:
        try:
            timed.wait(timeout=0.001)
            raise AssertionError("worker unexpectedly stopped before the startup timeout")
        except subprocess.TimeoutExpired:
            timed.kill()
            timed.wait(timeout=10)
        assert timed.returncode is not None
    finally:
        if timed.poll() is None:
            timed.kill()
            timed.wait()
    print(json.dumps({"offline_startup": "passed", "malformed_request": "passed", "stopped_worker": "passed", "startup_timeout": "passed"}))
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--check"]:
        raise SystemExit(run_check())
    if sys.argv[1:]:
        raise SystemExit("usage: laya_worker.py [--check]")
    raise SystemExit(serve())
