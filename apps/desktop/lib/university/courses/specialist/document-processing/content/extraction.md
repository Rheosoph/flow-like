You wire up extraction Tuesday morning and run the whole inbox. 190 contracts come back as clean markdown. 24 come back with no text at all. Same flow, same folder, no errors worth the name. Why?

> **Predict first:** What do those 24 files have in common — and did the extractor actually fail?

You already know the answer from lesson one: they're the office-scanner uploads. A deterministic reader opened each one, looked for a text layer, found a photograph instead, and returned exactly what it saw — nothing. That's not a crash. That's a routing signal.

## 1 · One node, many formats

The workhorse is **Extract Document**. Give it a file — PDF, DOCX, XLSX, even an image — and it converts the content to markdown. What comes out isn't one blob: it's an array of **pages**, each carrying its page number, its content as markdown, and any embedded images (there's an Extract Images toggle). Keeping pages separate preserves provenance — "the indemnity clause is on page 12" survives extraction.

When you do want one string, **Pages to Markdown** combines the array, inserting page markers along the way. And when you're processing the whole folder, **Extract Documents** accepts an array of files in a single node.

For PDFs specifically there's a surgical toolkit too: **Page Count** tells you how big the job is, **Split PDF** pulls a page range into a new file, and **Extract Text** grabs straight text from a digital PDF when you don't need pages or images.

## 2 · When rules aren't enough

The 24 scans need a different reader: **AI Extract Document**. Same job, same pages output — plus a model pin. You connect a vision-capable model, and the node lets it read what deterministic parsing can't: scanned pages, stamps, signatures, complex visual layouts. It handles OCR and image descriptions in one pass.

It also exposes honest engineering knobs. Pages Per Batch trades speed against memory. Images Per Message trades speed against token limits. Temperature defaults to 0.1 — extraction wants a model that transcribes, not one that improvises. The node is marked long-running for a reason: a 60-page scan is real work. A batch variant, **AI Extract Documents**, covers arrays.

One rule from the docs worth tattooing somewhere: pass the model as configuration. Don't hard-code a vendor-specific model name into a reusable board — model availability depends on what's configured, and the Models & Profiles course covers how that configuration works.

## 3 · Deterministic first

Why not send everything through the AI path "to be safe"? Three reasons, straight from the pipeline's economics:

- **Cost.** The deterministic reader is free. The AI path bills model calls per page — 214 contracts a week adds up.
- **Reproducibility.** Rules produce the same output every run. A model's transcription can vary — awkward when an auditor asks you to regenerate a record.
- **Privacy.** Deterministic extraction never shows the document to a model. For contracts, that's a feature you should spend deliberately, not by default.

So the pipeline routes: digital files take the cheap path, and only the files that *need* eyes get them. The empty-pages signal from the hook is exactly how you detect the split at runtime — extract deterministically, check whether content came back, and send the empties to the vision path.

**Watch out:** an empty result from a scan is the deterministic reader working correctly on a file with no text layer. Treat it as "route me differently," not "delete me" — those 24 scans are still signed contracts.

**Recap**

- Extract Document: one deterministic node, many formats, pages out.
- AI Extract Document adds a vision model for scans and layout-heavy files.
- Deterministic first — cheaper, reproducible, private; escalate only what needs it.
