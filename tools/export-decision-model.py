#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10,<3.14"
# dependencies = [
#   "gliner2 @ git+https://github.com/fastino-ai/GLiNER2.git@55656fbfa01d3d4a77485e1a1eeeaf682990ccdf",
#   "numpy>=1.26,<3",
#   "onnx>=1.18,<2",
#   "onnxruntime>=1.22,<2",
#   "onnxscript>=0.5,<1",
#   "peft>=0.18,<1",
#   "safetensors>=0.4,<1",
#   "torch>=2.8,<3",
#   "transformers>=5.17,<6",
# ]
# ///
"""Export a GLiNER2 classification bundle for Flow-Like's Typed Decision node.

Run with uv to install the dependencies in an isolated environment:
  uv run tools/export-decision-model.py --model fastino/gliner2.5-small-v1 --output ./decision-small

The output contains model.onnx, model.onnx_data, tokenizer.json, and
decision_config.json. The node uses raw logits at each [L] marker and applies
softmax over the candidate labels. Entity, relation, and record heads are omitted.

Exports run on the CPU in float32. Allow several times the checkpoint size in
memory and disk space; the 1B checkpoint needs multiple gigabytes. The destination
must not exist. It becomes visible only after ONNX Runtime and upstream parity
checks pass. Use --list-models to show the built-in checkpoints without loading one.
"""

from __future__ import annotations

import argparse
import gc
import json
import math
from pathlib import Path
import re
import shutil
import tempfile


MODELS = (
    "fastino/GLiNER2.5-Decide",
    "fastino/GLiNER2.5-multi-Decide",
    "fastino/GLiNER2.5-Decide-1B",
    "fastino/gliner2.5-multi-v1",
    "fastino/gliner2.5-base-v1",
    "fastino/gliner2.5-small-v1",
)

# Matches GLiNER2's WhitespaceTokenSplitter, including its punctuation tokens.
WORDS = re.compile(
    r"https?://[^\s]+|www\.[^\s]+|[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}"
    r"|@[a-z0-9_]+|\w+(?:[-_]\w+)*|\S",
    re.IGNORECASE,
)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    result.add_argument("--model", help="Hugging Face checkpoint ID or local checkpoint directory")
    result.add_argument("--output", type=Path, help="New bundle directory")
    result.add_argument("--list-models", action="store_true")
    result.add_argument("--revision", default="main", help="Checkpoint revision or commit (default: main)")
    result.add_argument("--max-length", type=int, default=512, help="Maximum sequence length, including the schema")
    result.add_argument("--exporter", choices=("legacy", "dynamo"), default="legacy")
    result.add_argument("--opset", type=int, default=18)
    return result


def encode_case(tokenizer, text: str, labels: dict[str, str]) -> tuple[list[int], list[int]]:
    """Recreate the node's per-part encoding and retain only structural markers."""
    prompt = "decision"
    for label, description in labels.items():
        prompt += f" [DESCRIPTION] {label}: {description}"
    parts = ["(", "[P]", prompt, "("]
    for label in labels:
        parts.extend(("[L]", label))
    parts.extend((")", ")", "[SEP_TEXT]"))
    if not text.endswith((".", "!", "?")):
        text += "."
    parts.extend(match.group().lower() for match in WORDS.finditer(text))

    ids, positions = [], []
    for index, part in enumerate(parts):
        if index in range(4, 4 + 2 * len(labels), 2):
            positions.append(len(ids))
        ids.extend(tokenizer.encode(part, add_special_tokens=False).ids)
    return ids, positions


def temperature_for(model) -> float:
    settings = getattr(model, "boundary_settings", None)
    value = float(getattr(settings, "classification_temperature", 1.0))
    if not math.isfinite(value) or value <= 0:
        raise ValueError("The checkpoint's classification temperature must be finite and positive")
    return value


def classifier_graph(model):
    import torch

    class Classifier(torch.nn.Module):
        def __init__(self, source):
            super().__init__()
            self.encoder = source.encoder
            self.classifier = source.classifier

        def forward(self, input_ids, attention_mask, label_positions):
            hidden = self.encoder(input_ids=input_ids, attention_mask=attention_mask).last_hidden_state
            markers = hidden.gather(1, label_positions.unsqueeze(-1).expand(-1, -1, hidden.shape[-1]))
            return self.classifier(markers).squeeze(-1).float()

    return Classifier(model).eval()


def reference_cases(model, graph, tokenizer, max_length: int, temperature: float):
    """Check tokenization, head selection, and probabilities against AutoExtractor."""
    import numpy as np
    import torch

    examples = [
        (
            ["Please cancel my subscription.", "Keep my account open, thanks!"],
            {"false": "Keep the subscription", "true": "Cancel the subscription"},
        ),
        (
            ["Überweisung fehlgeschlagen; contact help@example.com [L]"],
            {"0": "Billing or payment", "1": "Account access", "2": "Other requests mentioning [L]"},
        ),
    ]
    if max_length >= 256:
        labels = {
            "0": "The request remains unresolved", "1": "The customer is waiting",
            "2": "The issue is partly resolved", "3": "The issue is resolved",
        }
        sentence = "The support team resolved my problem and delivered the replacement on time."
        paragraphs = []
        # Cross ModernBERT's local attention window without exceeding the bundle budget.
        for _ in range(min(max_length, 384)):
            length = len(encode_case(tokenizer, " ".join(paragraphs + [sentence]), labels)[0])
            if length > min(max_length, 384):
                break
            paragraphs.append(sentence)
        if paragraphs:
            examples.append(([" ".join(paragraphs)], labels))
    cases = []
    for texts, labels in examples:
        # Returning all softmax scores checks every label, including low scores.
        schema = model.create_schema().classification(
            "decision", labels, multi_label=True, cls_threshold=0.0, class_act="softmax"
        )
        batch = model.processor.collate_fn_inference(
            [(text, schema) for text in texts], error_policy="raise"
        )
        positions = []
        for index, text in enumerate(texts):
            ids, marker_positions = encode_case(tokenizer, text, labels)
            count = int(batch.attention_mask[index].sum().item())
            if ids != batch.input_ids[index, :count].tolist():
                raise ValueError("Bundle tokenization differs from GLiNER2's processor")
            upstream_positions = list(batch.schema_special_indices[index][0][1:])
            if marker_positions != upstream_positions:
                raise ValueError("Classification markers differ from GLiNER2's processor")
            positions.append(marker_positions)
        if batch.input_ids.shape[1] > max_length:
            raise ValueError("--max-length is too short for the export validation examples")
        inputs = (batch.input_ids, batch.attention_mask, torch.tensor(positions, dtype=torch.int64))
        with torch.inference_mode():
            logits = graph(*inputs)
            expected = torch.softmax(logits / temperature, dim=-1).cpu().numpy()
            for index, text in enumerate(texts):
                result = model.extract(text, schema, include_confidence=True)["decision"]
                probabilities = {item["label"]: item["confidence"] for item in result}
                np.testing.assert_allclose(
                    expected[index], [probabilities[label] for label in labels],
                    rtol=2e-4, atol=2e-5,
                    err_msg="Export wrapper differs from upstream GLiNER2 classification",
                )
        cases.append((inputs, logits.detach().cpu().numpy()))
    return cases


def export(args) -> None:
    import numpy as np
    import onnx
    import onnxruntime as ort
    import torch
    from gliner2 import AutoExtractor
    from huggingface_hub import snapshot_download
    from tokenizers import Tokenizer

    destination = args.output.resolve()
    if destination.exists():
        raise FileExistsError(f"Destination already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    source = Path(args.model).expanduser()
    if source.is_dir():
        checkpoint = source.resolve()
        revision = None
    else:
        # Resolve once so the tokenizer, encoder config, and weights share a revision.
        checkpoint = Path(snapshot_download(
            args.model, revision=args.revision,
            allow_patterns=["*.json", "*.safetensors", "*.bin", "*.model", "*.txt"],
        ))
        revision = checkpoint.name
    print(f"Loading {checkpoint}", flush=True)
    model = AutoExtractor.from_pretrained(str(checkpoint), map_location="cpu", use_flashdeberta=False)
    model.float().eval()
    if getattr(model.processor, "token_pooling", "first") != "first":
        raise ValueError("Only checkpoints using first-token pooling are supported")
    encoder_config = model.encoder.config
    position_limit = getattr(encoder_config, "max_position_embeddings", args.max_length)
    if position_limit and args.max_length > position_limit:
        raise ValueError(f"--max-length exceeds the encoder limit of {position_limit}")
    # ModernBERT uses compiled helpers by default; export the ordinary eager path.
    if hasattr(encoder_config, "reference_compile"):
        encoder_config.reference_compile = False
    if hasattr(model.encoder, "set_attn_implementation"):
        model.encoder.set_attn_implementation("eager")
    temperature = temperature_for(model)
    architecture = model.config.architecture
    encoder_type = encoder_config.model_type
    graph = classifier_graph(model)

    with tempfile.TemporaryDirectory(prefix=f".{destination.name}-", dir=destination.parent) as temporary:
        staging = Path(temporary)
        bundle = staging / "bundle"
        bundle.mkdir()
        backend_tokenizer = getattr(model.processor.tokenizer, "backend_tokenizer", None)
        if backend_tokenizer is None:
            raise ValueError("A fast tokenizer with tokenizer.json support is required")
        backend_tokenizer.save(str(bundle / "tokenizer.json"))
        tokenizer = Tokenizer.from_file(str(bundle / "tokenizer.json"))
        tokenizer.no_padding()
        tokenizer.no_truncation()
        tokenizer.save(str(bundle / "tokenizer.json"))
        cases = reference_cases(model, graph, tokenizer, args.max_length, temperature)
        del model
        gc.collect()
        raw = staging / "raw"
        raw.mkdir()
        input_names = ["input_ids", "attention_mask", "label_positions"]
        axes = {"input_ids": {0: "batch", 1: "sequence"}, "attention_mask": {0: "batch", 1: "sequence"},
                "label_positions": {0: "batch", 1: "labels"}, "logits": {0: "batch", 1: "labels"}}
        export_options = {"dynamic_axes": axes, "do_constant_folding": False}
        if args.exporter == "dynamo":
            export_options = {"dynamic_shapes": {name: axes[name] for name in input_names}}
        print(f"Exporting {architecture}/{encoder_type} with {args.exporter}", flush=True)
        with torch.inference_mode():
            torch.onnx.export(
                graph, cases[0][0], str(raw / "model.onnx"),
                input_names=input_names, output_names=["logits"], opset_version=args.opset,
                dynamo=args.exporter == "dynamo", external_data=True, **export_options,
            )
        del graph
        gc.collect()

        # Use one predictable external-data name, including for checkpoints over 2 GB.
        exported = onnx.load(str(raw / "model.onnx"))
        onnx.save_model(
            exported, str(bundle / "model.onnx"), save_as_external_data=True,
            all_tensors_to_one_file=True, location="model.onnx_data", size_threshold=1024,
        )
        del exported
        gc.collect()
        shutil.rmtree(raw)
        onnx.checker.check_model(str(bundle / "model.onnx"))
        session = ort.InferenceSession(str(bundle / "model.onnx"), providers=["CPUExecutionProvider"])
        errors = []
        for inputs, expected in cases:
            feeds = {name: value.cpu().numpy() for name, value in zip(input_names, inputs)}
            actual, = session.run(["logits"], feeds)
            if actual.dtype != np.float32 or actual.shape != expected.shape:
                raise ValueError("Exported logits must be float32 with shape [batch, labels]")
            np.testing.assert_allclose(actual, expected, rtol=2e-4, atol=2e-4,
                                       err_msg="ONNX Runtime differs from the PyTorch classifier")
            errors.append(float(np.max(np.abs(actual - expected))))
        del session
        config = {
            "format": "gliner2-classifier", "max_length": args.max_length,
            "temperature": temperature, "source_model": args.model, "revision": revision,
            "external_data": ["model.onnx_data"] if (bundle / "model.onnx_data").exists() else [],
            "architecture": architecture, "encoder_type": encoder_type,
            "opset": args.opset, "validation_max_absolute_error": max(errors),
        }
        (bundle / "decision_config.json").write_text(json.dumps(config, indent=2) + "\n")
        bundle.rename(destination)
    print(f"Validated bundle saved to {destination}", flush=True)


def main() -> None:
    arguments = parser()
    args = arguments.parse_args()
    if args.list_models:
        print("\n".join(MODELS))
        return
    if not args.model or not args.output:
        arguments.error("model and output are required unless --list-models is used")
    if not 8 <= args.max_length <= 32768 or args.opset < 18:
        arguments.error("--max-length must be between 8 and 32768, and --opset must be at least 18")
    export(args)


if __name__ == "__main__":
    main()
