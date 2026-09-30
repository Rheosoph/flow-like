Optional: run pretrained OCR exports. Obtain compatible detection and recognition ONNX models, their preprocessing instructions and any required tokenizer before starting. Downloads/setup are outside the timed class.

Create two synthetic invoice images with `INV-1001` and `INV-1002` in your own editor. Store them in app Storage and complete the label sheet:

@VisionLabels

1. **Load ONNX** for each export. Use **Model Info** to inspect tensors and reproduce the model's documented preprocessing.
2. Run **Text Detection** on an invoice.
3. Feed regions and image through **Crop Text Regions**.
4. Run **Text Recognition** on the crops and compare the identifier with its label.
5. Repeat on the second image, then blur one copy and record what changes.

A line recognizer cannot be assumed to read a full page. Inspect crops when output is wrong. For runtime-named fields, **Zero-Shot NER (GLiNER)** provides another pretrained model path; fixed-label **Named Entity Recognition** requires its matching tokenizer.

Completion: both clean identifiers match. A degraded input is either correct or visibly routed to review. Preserve errors instead of rewriting expected labels. The [document-processing guide](https://docs.flow-like.com/topics/document-processing/overview/) covers a complete pipeline.
