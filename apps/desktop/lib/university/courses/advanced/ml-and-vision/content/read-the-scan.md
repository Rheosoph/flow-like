A customer replies to a billing ticket with a photographed invoice. Your classifier reads feature vectors; this is a JPEG of paper. Before the triage flow can route it, someone — or something — has to read the picture.

> **Predict first:** Is "read the text out of an image" one node, or a pipeline? And do you train that model yourself?

## 1 · Models trained elsewhere

You don't train vision models in Flow-Like. The ONNX family *runs* models that were trained elsewhere and exported to the ONNX format — you download a model file, load it with **Load ONNX** (which loads a model from a path), and run inference with task-specific nodes. The node descriptions even point at model sources: OCR detectors like CRAFT and DBNet, recognizers like CRNN and TrOCR, all published as ready-to-run exports.

Two disciplines replace training. First, **validate the model's contract**: the ONNX **Model Info** node reports the model's metadata plus its expected input and output tensors — check them before wiring anything, because a model fed tensors it doesn't expect fails or, worse, answers nonsense. Second, **reproduce the model's preprocessing** exactly as its documentation specifies; a pretrained model is only as good as the input format it was trained on.

Your fitted classifiers from lesson 4 and these ONNX models never mix artifacts: Save Model persists Flow-Like's own trained models, while ONNX models are files you manage in Storage like any other asset.

## 2 · The OCR relay

Reading a scanned invoice is a three-runner relay, not one node:

1. **Text Detection** finds *where* text lives in the image, returning text regions. Detector exports include CRAFT, DBNet, and EAST.
2. **Crop Text Regions** cuts each detected region out of the image, producing the small crops recognizers expect.
3. **Text Recognition** reads each crop into a string, using recognizer exports like CRNN, TrOCR, or PaddleOCR.

The order is load-bearing. Point Text Recognition at the full page and you've handed a line-reader a newspaper: recognizers are trained on cropped text snippets, not full layouts. Detection first, crop second, recognize third.

## 3 · From text to fields

Now you have strings — "Invoice No. 2024-1187", "Amount due: €430.00" — but the triage flow wants *fields*. That's named-entity recognition, and the catalog gives you two nodes with a clean division of labor:

**Named Entity Recognition** runs transformer-based NER models with a *fixed* label set — persons, organizations, locations, dates — with automatic tokenization. You download the model export plus its `tokenizer.json` from the same model repository; the tokenizer is not optional, it's how text becomes the tensors the model expects.

**Zero-Shot NER (GLiNER)** extracts entities for *any labels you name at runtime* — no fixed label set, no retraining. Want `invoice_number`, `amount`, and `due_date`? Type them in as the labels. Its own description draws the boundary: for models with a fixed label set, use the Named Entity Recognition node instead.

For the invoice case, GLiNER is the fit: no pretrained fixed-set model ships with your labels, and you can't fine-tune one here — but you can *name* what you're looking for.

**Watch out:** OCR output is evidence, not truth. A recognizer misreads a smudged 8 as 3 without blushing. Before amounts flow into anything that matters, validate formats in the flow — and for full compliance pipelines with PII redaction, the Document Processing course owns that ground.

**Recap:**

- ONNX nodes run pretrained exported models — Load ONNX from a path, verify tensors with Model Info, reproduce the documented preprocessing.
- OCR is a relay: Text Detection → Crop Text Regions → Text Recognition.
- Fixed labels → Named Entity Recognition (with its tokenizer.json); labels you invent at runtime → Zero-Shot NER (GLiNER).
