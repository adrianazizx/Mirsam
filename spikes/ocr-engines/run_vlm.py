"""Vision-language OCR models over the evaluation set, through 🤗 Transformers (ADR 0030).

    python run_vlm.py <model> [--only <id substring>] [--device mps|cpu] [--confidence]

With --confidence the run goes to hyp/<model>+conf/ and also writes
<sample>.conf.json: every word of the raw answer with the lowest probability
the model gave any of its tokens (greedy decoding, so the chosen token's softmax).

Each model runs the way its card documents: its own prompt, its own input size,
greedy decoding. Output markup is removed by `to_text.py`, identically for all.
Weights come from the Hugging Face cache under SAFHA_OCR_BENCH_CACHE/hf.
"""

from __future__ import annotations

import argparse
import json
import os
import time
from pathlib import Path

from common import CACHE, Run, load_samples

os.environ.setdefault("HF_HOME", str(CACHE / "hf"))
os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")

import torch  # noqa: E402
from PIL import Image  # noqa: E402
from transformers import AutoModelForImageTextToText, AutoProcessor  # noqa: E402

from to_text import to_text  # noqa: E402

MAX_NEW_TOKENS = 4096
TWO_POINT_FOUR_MP = 2_400_000

QARI_03_PROMPT = (
    "Below is the image of one page of a document, as well as some raw textual content that was "
    "previously extracted for it. Just return the plain text representation of this document as if "
    "you were reading it naturally. Do not hallucinate."
)
NUQTA_PROMPT = (
    "Transcribe the Arabic text in this image exactly as written. "
    "Include only the diacritics (tashkeel) that are visible in the image."
)


def cap(image: Image.Image, limit: int = TWO_POINT_FOUR_MP) -> Image.Image:
    """Shrinks an image to at most `limit` pixels (aspect kept)."""
    w, h = image.size
    if w * h <= limit:
        return image
    k = (limit / (w * h)) ** 0.5
    return image.resize((max(32, int(w * k)), max(32, int(h * k))), Image.LANCZOS)


def nuqta_prepare(image: Image.Image) -> Image.Image:
    """The card's recommended input size: pages to ~2.4 MP, crops as they are, all capped at 2.4 MP."""
    w, h = image.size
    page = max(w, h) / min(w, h) <= 2 and min(w, h) >= 450
    if page or w * h > TWO_POINT_FOUR_MP:
        k = (TWO_POINT_FOUR_MP / (w * h)) ** 0.5
        image = image.resize((max(32, int(w * k)), max(32, int(h * k))), Image.LANCZOS)
    return image


# repo, revision-free (the cache records the commit), prompt, preprocessing, extra template kwargs
MODELS = {
    # Qwen2-VL-2B fine-tune. The card uses the processor's default pixel range, whose
    # ceiling (12.8 MP) would put ~44k patches through full attention on a 300 dpi page;
    # pages are capped at 2.4 MP, the same cap as the other Qwen-family models here.
    "qari-0.3": dict(repo="NAMAA-Space/Qari-OCR-v0.3-VL-2B-Instruct", prompt=QARI_03_PROMPT, prepare=cap),
    # LoRA on Qwen3-VL-4B-Instruct; "Free OCR." is the card's prompt.
    "qari-0.4": dict(
        repo="Qwen/Qwen3-VL-4B-Instruct",
        adapter="NAMAA-Space/Qari-OCR-0.4.0-VL-4B-Instruct",
        prompt="Free OCR.",
        prepare=cap,
    ),
    # Element-level "OCR:" task on the whole page, at the card's pixel ceiling for its
    # largest task (spotting, 1.6 MP). Page-level parsing with layout needs the
    # PaddleOCR pipeline (run_paddleocr_vl.py).
    "paddleocr-vl-1.6": dict(
        repo="PaddlePaddle/PaddleOCR-VL-1.6",
        prompt="OCR:",
        prepare=lambda image: image,
        images_kwargs={"size": {"shortest_edge": 112 * 112, "longest_edge": 2048 * 28 * 28}},
    ),
    "nuqta-4b": dict(
        repo="SalahALHaismawi/Nuqta-Arabic-OCR-4B",
        prompt=NUQTA_PROMPT,
        prepare=nuqta_prepare,
        template_kwargs={"enable_thinking": False},
    ),
    "nuqta-0.8b": dict(
        repo="SalahALHaismawi/Nuqta-Arabic-OCR-0.8B",
        prompt=NUQTA_PROMPT,
        prepare=nuqta_prepare,
        template_kwargs={"enable_thinking": False},
    ),
}


def word_confidences(processor, tokens, scores) -> list[list]:
    """[word, lowest token probability in it] for each whitespace-separated word of the answer."""
    probs = [torch.softmax(step[0].float(), dim=-1)[token].item() for step, token in zip(scores, tokens)]
    text, char_probs = "", []
    for i in range(len(tokens)):
        decoded = processor.decode(tokens[: i + 1], skip_special_tokens=True)
        char_probs += [probs[i]] * max(0, len(decoded) - len(text))
        text = decoded
    words, current, low = [], "", 1.0
    for char, prob in zip(text, char_probs):
        if char.isspace():
            if current:
                words.append([current, round(low, 4)])
            current, low = "", 1.0
        else:
            current += char
            low = min(low, prob)
    if current:
        words.append([current, round(low, 4)])
    return words


def weights_bytes(repo: str) -> int:
    """On-disk size of a cached model repo (the blobs its snapshot links to)."""
    from huggingface_hub import snapshot_download

    snapshot = Path(snapshot_download(repo, local_files_only=True))
    return sum(p.resolve().stat().st_size for p in snapshot.rglob("*") if p.is_file())


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("model", choices=sorted(MODELS))
    parser.add_argument("--only")
    parser.add_argument("--device", default="mps" if torch.backends.mps.is_available() else "cpu")
    parser.add_argument("--dtype", default="bfloat16", choices=["bfloat16", "float16", "float32"])
    parser.add_argument("--confidence", action="store_true")
    args = parser.parse_args()
    spec = MODELS[args.model]
    dtype = getattr(torch, args.dtype)

    started = time.time()
    # Qari 0.4's card loads the processor from the adapter repo; it is Qwen3-VL's.
    processor = AutoProcessor.from_pretrained(spec.get("adapter", spec["repo"]))
    model = AutoModelForImageTextToText.from_pretrained(spec["repo"], dtype=dtype)
    model_bytes = weights_bytes(spec["repo"])
    if "adapter" in spec:
        from peft import PeftModel

        model = PeftModel.from_pretrained(model, spec["adapter"]).merge_and_unload()
        model_bytes += weights_bytes(spec["adapter"])
    model = model.to(args.device).eval()

    import transformers

    run = Run(
        f"{args.model}+conf" if args.confidence else args.model,
        {
            "repo": spec["repo"],
            "adapter": spec.get("adapter"),
            "prompt": spec["prompt"],
            "device": args.device,
            "dtype": args.dtype,
            "decoding": f"greedy, KV cache, max_new_tokens={MAX_NEW_TOKENS}",
            "transformers": transformers.__version__,
            "torch": torch.__version__,
        },
    )
    run.load_seconds = time.time() - started
    mps = args.device == "mps"

    for sample in load_samples(args.only):
        image = spec["prepare"](Image.open(sample.image).convert("RGB"))
        messages = [{"role": "user", "content": [{"type": "image", "image": image}, {"type": "text", "text": spec["prompt"]}]}]
        t0 = time.time()
        inputs = processor.apply_chat_template(
            messages,
            add_generation_prompt=True,
            tokenize=True,
            return_dict=True,
            return_tensors="pt",
            **spec.get("template_kwargs", {}),
            **({"images_kwargs": spec["images_kwargs"]} if "images_kwargs" in spec else {}),
        ).to(args.device)
        with torch.inference_mode():
            # PaddleOCR-VL-1.6 ships `use_cache: false` in its generation config, which makes
            # Transformers re-run the whole image prefix per token and end answers early.
            out = model.generate(
                **inputs,
                max_new_tokens=MAX_NEW_TOKENS,
                do_sample=False,
                use_cache=True,
                output_scores=args.confidence,
                return_dict_in_generate=True,
            )
        new_tokens = out.sequences[0, inputs["input_ids"].shape[1] :]
        raw = processor.decode(new_tokens, skip_special_tokens=True)
        if mps:
            torch.mps.synchronize()
            run.extra_peak_bytes = max(run.extra_peak_bytes, torch.mps.driver_allocated_memory())
        seconds = time.time() - t0
        flags = {}
        if len(new_tokens) >= MAX_NEW_TOKENS:
            flags["hit_token_limit"] = True
        (run.hyp_dir / f"{sample.id}.raw.txt").write_text(raw, encoding="utf-8")
        if args.confidence:
            confidences = word_confidences(processor, new_tokens.tolist(), out.scores)
            (run.hyp_dir / f"{sample.id}.conf.json").write_text(json.dumps(confidences, ensure_ascii=False))
        run.write(sample, to_text(raw), seconds, **flags)
        if mps:
            torch.mps.empty_cache()
    run.finish(model_bytes)


if __name__ == "__main__":
    main()
