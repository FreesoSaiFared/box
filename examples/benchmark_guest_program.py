#!/usr/bin/env python3
"""Reproducible HTTP comparison: N discrete exec requests vs one mixed recipe.
Tests only request count, elapsed time and explicit result verification.
Does NOT claim model-token savings (no model runs in this harness).
"""
import argparse
import json
import os
import statistics
import time
import urllib.error
import urllib.request
from pathlib import Path


def post(base, token, route, data):
    wire = json.dumps(data, separators=(",", ":")).encode()
    req = urllib.request.Request(
        base + route, wire, method="POST",
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp)


def one_run(base, token, count, mode):
    started = time.perf_counter()
    if mode == "discrete":
        for i in range(count):
            result = post(base, token, "/v1/exec", {
                "command": ["/bin/sh", "-c", f"printf 'step-{i}'"]
            })
            assert result["exit_code"] == 0 and result["stdout"] == f"step-{i}", result
        requests = count
        explicit_assertions = count
    elif mode == "compiled":
        steps = []
        for i in range(count):
            steps.append({"op": "exec", "id": f"e{i}",
                          "command": ["/bin/sh", "-c", f"printf 'step-{i}'"]})
            steps.append({"op": "assert", "id": f"v{i}", "from": f"e{i}",
                          "pointer": "/stdout", "equals": f"step-{i}"})
        result = post(base, token, "/v1/recipe", {
            "name": "benchmark", "steps": steps,
        })
        assert result["ok"], result
        assert result["assertions_passed"] == count, result
        requests = 1
        explicit_assertions = result["assertions_passed"]
    else:
        raise ValueError(mode)
    return {
        "mode": mode, "http_requests": requests,
        "assertions": explicit_assertions,
        "elapsed_ms": round((time.perf_counter() - started) * 1000, 3)
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", default=os.getenv("BOX_EXEC_URL", "http://127.0.0.1:1337"))
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--steps", type=int, default=6)
    ap.add_argument("--out", default="")
    a = ap.parse_args()
    if a.steps < 1 or a.steps > 120 or a.runs < 1:
        ap.error("steps must be 1..120; runs must be positive")
    token = os.environ["BOX_TOKEN"]
    samples = []
    for _ in range(a.runs):
        for mode in ("discrete", "compiled"):
            samples.append(one_run(a.base_url.rstrip("/"), token, a.steps, mode))
    report = {
        "schema": "GUEST_PROGRAM_BENCH/1", "host_endpoint": a.base_url,
        "runs": a.runs, "steps": a.steps,
        "model_calls_measured": False,
        "samples": samples,
        "median_ms": {
            mode: round(statistics.median(r["elapsed_ms"] for r in samples if r["mode"] == mode), 3)
            for mode in ("discrete", "compiled")
        },
        "claims": [
            "mechanical work completed and explicit stdout assertions passed",
            "no semantic truth beyond the stated assertions is inferred",
            "request count is measured here, model tokens are not",
        ]
    }
    if a.out:
        path = Path(a.out)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
