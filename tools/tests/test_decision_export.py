"""Check the decision exporter without downloading model weights.

The ModernBERT test uses the export script's Python dependencies. It is skipped
when those dependencies are absent. CLI validation needs only the standard library.
"""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "export-decision-model.py"
SPEC = importlib.util.spec_from_file_location("decision_export", SCRIPT)
EXPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EXPORT)
HAS_EXPORT_DEPS = all(importlib.util.find_spec(name) is not None for name in (
    "torch", "transformers", "onnx", "onnxruntime", "numpy"
))


class DecisionExportTests(unittest.TestCase):
    def run_cli(self, *arguments):
        # -S removes site packages so help and validation cannot rely on ML dependencies.
        return subprocess.run([sys.executable, "-S", str(SCRIPT), *arguments],
                              capture_output=True, text=True)

    def test_cli_inspection_does_not_load_models(self):
        for flag in ("--help", "--list-models"):
            with self.subTest(flag=flag):
                result = self.run_cli(flag)
                self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.run_cli("--list-models").stdout.splitlines(), list(EXPORT.MODELS))

    def test_invalid_budget_fails_before_loading_dependencies(self):
        for budget in ("0", "7", "32769"):
            with self.subTest(budget=budget):
                result = self.run_cli("--model", "unused", "--output", "unused",
                                      "--max-length", budget)
                self.assertEqual(result.returncode, 2)
                self.assertIn("between 8 and 32768", result.stderr)
                self.assertNotIn("ModuleNotFoundError", result.stderr)

    @unittest.skipUnless(HAS_EXPORT_DEPS, "install the decision export dependencies")
    def test_modernbert_graph_supports_dynamic_shapes_and_padding(self):
        import numpy as np
        import onnxruntime as ort
        import torch
        from transformers import ModernBertConfig, ModernBertModel

        torch.manual_seed(11)
        config = ModernBertConfig(
            vocab_size=64, hidden_size=32, intermediate_size=48,
            num_hidden_layers=3, num_attention_heads=4, max_position_embeddings=512,
            layer_types=["full_attention", "sliding_attention", "sliding_attention"],
            local_attention=8, pad_token_id=0, bos_token_id=1, eos_token_id=2,
            cls_token_id=1, sep_token_id=2,
        )
        config._attn_implementation = "eager"
        graph = EXPORT.classifier_graph(SimpleNamespace(
            encoder=ModernBertModel(config), classifier=torch.nn.Linear(32, 1)
        ))
        inputs = (torch.randint(1, 64, (2, 20)), torch.ones((2, 20), dtype=torch.int64),
                  torch.tensor([[3, 5], [3, 5]]))
        inputs[0][1, 15:] = 0
        inputs[1][1, 15:] = 0
        changed = (torch.randint(1, 64, (1, 28)), torch.ones((1, 28), dtype=torch.int64),
                   torch.tensor([[3, 6, 8]]))
        names = ["input_ids", "attention_mask", "label_positions"]
        axes = {"input_ids": {0: "batch", 1: "sequence"},
                "attention_mask": {0: "batch", 1: "sequence"},
                "label_positions": {0: "batch", 1: "labels"},
                "logits": {0: "batch", 1: "labels"}}
        with tempfile.TemporaryDirectory() as temporary, torch.inference_mode():
            path = str(Path(temporary) / "model.onnx")
            torch.onnx.export(
                graph, inputs, path, input_names=names, output_names=["logits"],
                opset_version=18, dynamo=False, external_data=True,
                dynamic_axes=axes, do_constant_folding=False,
            )
            session = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
            for sample in (inputs, changed):
                actual, = session.run(["logits"], dict(zip(names, (x.numpy() for x in sample))))
                expected = graph(*sample).numpy()
                self.assertEqual(actual.dtype, np.float32)
                self.assertEqual(actual.shape, tuple(sample[2].shape))
                np.testing.assert_allclose(actual, expected, rtol=2e-4, atol=2e-4)


if __name__ == "__main__":
    unittest.main()
