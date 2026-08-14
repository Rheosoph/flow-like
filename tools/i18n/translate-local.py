#!/usr/bin/env python3
"""Small JSON-in/JSON-out worker for a local Marian translation model."""

import argparse
import json
import sys

import torch
from transformers import MarianMTModel, MarianTokenizer


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--batch-size", type=int, default=48)
    args = parser.parse_args()

    texts = json.load(sys.stdin)
    if not isinstance(texts, list) or not all(isinstance(item, str) for item in texts):
        raise ValueError("stdin must be a JSON array of strings")

    tokenizer = MarianTokenizer.from_pretrained(args.model, local_files_only=True)
    model = MarianMTModel.from_pretrained(args.model, local_files_only=True)
    model.eval()
    results = [""] * len(texts)

    # Similar lengths in each batch reduce padding and make CPU inference faster.
    indices = sorted(range(len(texts)), key=lambda index: len(texts[index]))
    for start in range(0, len(indices), args.batch_size):
        batch_indices = indices[start : start + args.batch_size]
        batch = [texts[index] for index in batch_indices]
        inputs = tokenizer(
            batch,
            return_tensors="pt",
            padding=True,
            truncation=True,
            max_length=512,
        )
        with torch.inference_mode():
            generated = model.generate(
                **inputs,
                max_new_tokens=512,
                num_beams=1,
            )
        decoded = tokenizer.batch_decode(generated, skip_special_tokens=True)
        for index, value in zip(batch_indices, decoded, strict=True):
            results[index] = value
        completed = min(start + args.batch_size, len(indices))
        print(f"local model: {completed}/{len(indices)}", file=sys.stderr, flush=True)

    json.dump(results, sys.stdout, ensure_ascii=False)


if __name__ == "__main__":
    main()
