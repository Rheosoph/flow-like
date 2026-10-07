#!/usr/bin/env python3
"""Generate EmbeddingGemma 2 oracle vectors with pinned upstream processors.

Requires numpy, pillow, torch, torchvision, transformers, tokenizers, onnxruntime.
Run with --assets pointing to the pinned ONNX repository checkout. This script
downloads the four processor source files from their immutable upstream commit.
"""

import argparse
import hashlib
import importlib.metadata
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import urllib.request

os.environ["TOKENIZERS_PARALLELISM"] = "false"

import numpy as np
import onnxruntime as ort
from PIL import Image
from transformers import AutoTokenizer

SOURCE_REVISION = "14e738b5d0cc69aa27a95dde272aea41fde44f2f"
MODEL_REVISION = "daa72c51243991dfcaf9f9137d2c573d8f7790c0"
MODEL_FILES_SHA256 = {
    "config.json": "8d011bfe08b5e345bbe0b81e5c6fd02c381920b345b986047bc2a33ce7b90d1d",
    "processor_config.json": "168f6a08522f3ce5dea596d94d003af2fd691742d4f41fe1f9d8cce76bfbf69c",
    "tokenizer.json": "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4",
    "tokenizer_config.json": "17bd5d6e9364ca49a534e1502076593317c298d4a663623091ed45388f004874",
    "onnx/model_q4.onnx": "f9eeba97acddf139b8ee2ddf04bc30dceafa88de93fadf74d7644e0d61a477a9",
    "onnx/model_q4.onnx_data": "c3975f2d1ab7a1878ae31a7d7a9b7804a827aff3800b60dfceafce21cac3df49",
    "onnx/vision_encoder_q4.onnx": "7ea284226d4938f0ad921ab091f1d80a9ca699aa802984ef5cd5eec4f4761d96",
    "onnx/vision_encoder_q4.onnx_data": "0a9d6c927334f152a33dd90874f65d6ea5228999abe6a450d3f7813677fa704c",
    "onnx/audio_encoder_q4.onnx": "c4cce3370e72262280d00293cac038896050a03a8e9a27e10b061ca97510296e",
    "onnx/audio_encoder_q4.onnx_data": "ba9328e6341360974083085b44b2bba265003bda564740f7c4c23ed9928f17e2"
}
SOURCE_FILES = {
    "image": "gemma4/image_processing_gemma4.py",
    "audio": "gemma4/feature_extraction_gemma4.py",
    "video": "embedding_gemma2/video_processing_embedding_gemma2.py",
    "processor": "embedding_gemma2/processing_embedding_gemma2.py",
}


def official_modules(directory):
    modules, hashes = {}, {}
    for role, relative in SOURCE_FILES.items():
        url = (
            "https://raw.githubusercontent.com/huggingface/transformers/"
            f"{SOURCE_REVISION}/src/transformers/models/{relative}"
        )
        data = urllib.request.urlopen(url).read()
        hashes[relative] = hashlib.sha256(data).hexdigest()
        path = directory / Path(relative).name
        path.write_bytes(data)
        module_name = "transformers.models." + relative.removesuffix(".py").replace("/", ".") + "_reference"
        spec = importlib.util.spec_from_file_location(module_name, path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        spec.loader.exec_module(module)
        modules[role] = module
    return modules, hashes


def picture(phase=0):
    y, x = np.indices((64, 80))
    pixels = np.stack(
        [(x * 3 + y + phase) % 256, (y * 4 + phase) % 256, (x + y * 2 + phase) % 256],
        axis=-1,
    ).astype(np.uint8)
    return Image.fromarray(pixels)


def cases():
    i = np.arange(16000, dtype=np.float64)
    wave = (
        0.2 * np.sin(2 * np.pi * 440 * i / 16000)
        + 0.05 * np.cos(2 * np.pi * 1379 * i / 16000)
    ).astype(np.float32)
    video = np.stack([np.array(picture(0)), np.array(picture(51))], axis=0)
    return [
        {"text": ["title: none | text: A red fox jumps over a log."]},
        {"text": ["<|image|>"], "images": [[picture()]]},
        {"text": ["<|audio|>"], "audio": [wave]},
        {"text": ["<|video|>"], "videos": [video]},
        {
            "text": ["title: none | text: A colorful scene <|image|> with sound <|audio|><|video|>"],
            "images": [[picture()]], "audio": [wave], "videos": [video],
        },
        {"text": ["title: none | text: A colorful scene <|image|>"], "images": [[picture()]]},
        {"text": ["title: none | text: A recorded sound <|audio|>"], "audio": [wave]},
        {"text": ["title: none | text: A colorful video <|video|>"], "videos": [video]},
        {"text": ["<|video|><|audio|>"], "videos": [video], "audio": [wave]},
    ]


def sampled(values):
    flat = np.asarray(values).reshape(-1)
    indices = np.linspace(0, len(flat) - 1, min(1024, len(flat)), dtype=int)
    return {"length": len(flat), "indices": indices.tolist(), "values": flat[indices].tolist()}


def generate(root, modules, hashes):
    for name, expected in MODEL_FILES_SHA256.items():
        with (root / name).open("rb") as asset:
            actual = hashlib.file_digest(asset, "sha256").hexdigest()
        if actual != expected:
            raise ValueError(f"{name} does not match pinned model revision {MODEL_REVISION}")
    config = json.loads((root / "processor_config.json").read_text())
    image_processor = modules["image"].Gemma4ImageProcessor(**config["image_processor"])
    audio_processor = modules["audio"].Gemma4AudioFeatureExtractor(**config["feature_extractor"])
    video_processor = modules["video"].EmbeddingGemma2VideoProcessor(**config["video_processor"])
    tokenizer = AutoTokenizer.from_pretrained(root, local_files_only=True)
    processor = modules["processor"].EmbeddingGemma2Processor(
        feature_extractor=audio_processor,
        image_processor=image_processor,
        tokenizer=tokenizer,
        video_processor=video_processor,
        image_seq_length=280,
        audio_seq_length=280,
        audio_ms_per_token=40,
    )
    options = ort.SessionOptions()
    options.intra_op_num_threads = 4
    sessions = {
        name: ort.InferenceSession(
            str(root / f"onnx/{name}_q4.onnx"),
            sess_options=options,
            providers=["CPUExecutionProvider"],
        )
        for name in ["model", "vision_encoder", "audio_encoder"]
    }
    result = {
        "source_revision": SOURCE_REVISION,
        "source_sha256": hashes,
        "model_repository": "onnx-community/embeddinggemma-2-ONNX",
        "model_revision": MODEL_REVISION,
        "dtype": "q4",
        "model_sha256": MODEL_FILES_SHA256,
        "versions": {
            package: importlib.metadata.version(package)
            for package in ["numpy", "pillow", "torch", "torchvision", "transformers", "tokenizers", "onnxruntime"]
        },
        "embeddings": [],
    }
    for index, case in enumerate(cases()):
        features = processor(**case, return_tensors="np", videos_kwargs={"do_sample_frames": False})
        inputs = {
            "input_ids": np.asarray(features["input_ids"], dtype=np.int64),
            "attention_mask": np.asarray(features["attention_mask"], dtype=np.int64),
            "image_features": np.zeros((0, 512), np.float32),
            "video_features": np.zeros((0, 512), np.float32),
            "audio_features": np.zeros((0, 512), np.float32),
        }
        for pixels, positions, destination in [
            ("pixel_values", "image_position_ids", "image_features"),
            ("pixel_values_videos", "video_position_ids", "video_features"),
        ]:
            if pixels in features:
                inputs[destination] = sessions["vision_encoder"].run(None, {
                    "pixel_values": features[pixels].astype(np.float32),
                    "pixel_position_ids": features[positions].astype(np.int64),
                })[0]
        if "input_features" in features:
            inputs["audio_features"] = sessions["audio_encoder"].run(None, {
                "input_features": features["input_features"].astype(np.float32),
                "input_features_mask": features["input_features_mask"].astype(bool),
            })[0]
        if index == 1:
            result["image_pixels"] = sampled(features["pixel_values"])
        if index == 2:
            result["audio_features"] = sampled(features["input_features"])
        vector = sessions["model"].run(["sentence_embedding"], inputs)[0][0]
        result["embeddings"].append((vector / np.linalg.norm(vector)).tolist())
        print(f"Generated fixture {index + 1}/9", flush=True)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--output", type=Path, default=Path(__file__).parent / "fixtures/gemma2/reference.json")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="gemma2-reference-") as directory:
        modules, hashes = official_modules(Path(directory))
        reference = generate(args.assets, modules, hashes)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(reference, separators=(",", ":")) + "\n")
