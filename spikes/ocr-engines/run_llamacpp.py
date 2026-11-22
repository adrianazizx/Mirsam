"""Nuqta through llama.cpp (GGUF), the runtime the app could ship (ADR 0030, M4 runtime re-measure).

Starts `llama-server` once per run (as an app would), sends one page per request with the
model card's prompt, input size and flags, and records wall time per page, peak server RSS and
the server's own prompt/generation token timings.

    run_llamacpp.py nuqta-4b --quant Q4_K_M [--cpu] [--only form] [--threads N]

Engine name: nuqta-<size>-<quant>-<metal|cpu>, e.g. nuqta-4b-q4_k_m-metal.
"""

from __future__ import annotations

import argparse
import base64
import io
import json
import os
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))
from common import CACHE, Run, load_samples  # noqa: E402
from run_vlm import NUQTA_PROMPT, nuqta_prepare  # noqa: E402
from to_text import to_text  # noqa: E402

REPOS = {"nuqta-4b": "Nuqta-Arabic-OCR-4B-GGUF", "nuqta-0.8b": "Nuqta-Arabic-OCR-0.8B-GGUF"}
PORT = 8791
MAX_TOKENS = 4096


def gguf_dir(model: str) -> Path:
    base = CACHE / "hf" / "hub" / f"models--SalahALHaismawi--{REPOS[model]}" / "snapshots"
    return next(base.iterdir())


def rss_mib(pid: int) -> int:
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return int(out) // 1024 if out else 0


def post(payload: dict) -> dict:
    request = urllib.request.Request(
        f"http://127.0.0.1:{PORT}/v1/chat/completions",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=7200) as response:
        return json.load(response)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("model", choices=sorted(REPOS))
    parser.add_argument("--quant", default="Q4_K_M", choices=["Q4_K_M", "Q8_0"])
    parser.add_argument("--cpu", action="store_true", help="no GPU at all (-ngl 0, vision encoder on the CPU)")
    parser.add_argument("--threads", type=int)
    parser.add_argument("--only")
    parser.add_argument("--limit", type=int, help="first N samples only")
    parser.add_argument("--stride", type=int, default=1, help="every Nth sample (CPU speed runs)")
    parser.add_argument("--tag", default="")
    args = parser.parse_args()

    short = args.model.replace("nuqta-", "")
    directory = gguf_dir(args.model)
    model_file = directory / f"nuqta-ocr-{short}-{args.quant}.gguf"
    mmproj = directory / f"nuqta-ocr-{short}-mmproj-f16.gguf"
    cmd = [
        shutil.which("llama-server") or "llama-server",
        "-m", str(model_file), "--mmproj", str(mmproj),
        "-c", "8192", "--image-max-tokens", "4096", "--temp", "0",
        "--port", str(PORT), "--no-webui", "--parallel", "1",
    ]
    if args.cpu:
        cmd += ["-ngl", "0", "--no-mmproj-offload"]
    if args.threads:
        cmd += ["-t", str(args.threads)]
    backend = "cpu" if args.cpu else "metal"
    name = f"{args.model}-{args.quant.lower()}-{backend}{args.tag}"

    log = open(CACHE / f"llama-server-{name}.log", "w")
    started = time.time()
    server = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    try:
        while True:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2)
                break
            except Exception:
                if server.poll() is not None:
                    sys.exit(f"llama-server exited early; see {log.name}")
                time.sleep(1)
        load = time.time() - started

        peak = [0]
        stop = threading.Event()

        def watch() -> None:
            while not stop.is_set():
                peak[0] = max(peak[0], rss_mib(server.pid))
                time.sleep(0.5)

        threading.Thread(target=watch, daemon=True).start()

        run = Run(name, {
            "runtime": "llama-server (llama.cpp 1.0.0, build 12359)",
            "model_file": model_file.name,
            "mmproj": mmproj.name,
            "backend": backend,
            "command": " ".join(cmd[1:]).replace(str(directory), "<gguf>"),
            "prompt": NUQTA_PROMPT,
            "decoding": f"greedy (temp 0), max_tokens={MAX_TOKENS}",
        })
        run.load_seconds = load
        samples = load_samples(args.only)
        samples = samples[:: args.stride]
        if args.limit:
            samples = samples[: args.limit]
        timings = {}
        for sample in samples:
            image = nuqta_prepare(Image.open(sample.image).convert("RGB"))
            buffer = io.BytesIO()
            image.save(buffer, "PNG")
            data_url = "data:image/png;base64," + base64.b64encode(buffer.getvalue()).decode()
            payload = {
                "messages": [{"role": "user", "content": [
                    {"type": "image_url", "image_url": {"url": data_url}},
                    {"type": "text", "text": NUQTA_PROMPT},
                ]}],
                "temperature": 0,
                "max_tokens": MAX_TOKENS,
                "chat_template_kwargs": {"enable_thinking": False},
            }
            t0 = time.time()
            reply = post(payload)
            seconds = time.time() - t0
            raw = reply["choices"][0]["message"]["content"] or ""
            t = reply.get("timings", {})
            timings[sample.id] = {
                "prompt_tokens": t.get("prompt_n"), "prompt_ms": round(t.get("prompt_ms", 0)),
                "generated_tokens": t.get("predicted_n"), "generation_ms": round(t.get("predicted_ms", 0)),
            }
            flags = {}
            if reply["choices"][0].get("finish_reason") == "length":
                flags["hit_token_limit"] = True
            (run.hyp_dir / f"{sample.id}.raw.txt").write_text(raw, encoding="utf-8")
            run.write(sample, to_text(raw), seconds, **flags)
        stop.set()
        run.settings["server_timings"] = timings
        run.finish(model_file.stat().st_size + mmproj.stat().st_size)
        # peak_rss_mib in finish() is this script's; replace it with the server's.
        path = run.hyp_dir / "run.json"
        record = json.loads(path.read_text())
        record["peak_rss_mib"] = peak[0]
        record["server_timings"] = record["settings"].pop("server_timings")
        path.write_text(json.dumps(record, indent=2, ensure_ascii=False) + "\n")
        print("server peak RSS MiB:", peak[0])
    finally:
        server.terminate()
        server.wait()


if __name__ == "__main__":
    main()
