Wednesday afternoon, the first redacted NDA goes out to the wider team. Ten minutes later, someone replies: "Paragraph 12 still has the signer's full name." The mask ran. It reported detections. It even redacted two email addresses in the same document. So why is a person's name sitting in the shared copy?

> **Predict first:** What kind of PII can a regular expression catch — and what kind slips straight through it?

## 1 · The pattern mask

**PII Mask (Regex)** catches PII that has a *shape*: emails, phone numbers, SSNs, credit cards, IBANs, US/DE/UK addresses. Shapes are what regular expressions are made of, which is why this node is fast, deterministic, free, and private — the text never leaves the runtime.

It gives you three outputs worth keeping: **Masked Text** (the redacted document), **Detection Count** (how many hits), and **Detections** — a JSON array recording each hit's type, position, and length. That array is your audit trail: proof of what was found and where, without repeating the sensitive values themselves.

You control what it hunts for with per-type toggles, or centrally with a **PII Detection Options** node — configure once, connect to every mask in the app.

And now the hook resolves: a *name* has no shape. "Sarah Okonkwo" and "Master Services Agreement" look identical to a pattern matcher — capitalized words. The regex node's own description says it plainly: for names or contextual PII, use the AI-based node.

## 2 · The context mask

**PII Mask (AI)** connects a model that reads for meaning. It catches names, addresses written in prose, and indirect references like "our CFO, who signed below" — things no pattern will ever match. You can set the replacement text (default `[REDACTED]`), choose a sensitivity level, and steer detection with a context instruction: "focus on medical records," "also mask company names."

Two trade-offs to spend deliberately. It bills model calls, so it's not free at 214 documents a week. And the text is shown to whatever model you've configured — for contracts, choose that model consciously rather than by default. The Models & Profiles course covers how model configuration works.

## 3 · Layer them

The production pattern is both, in order: regex first — it deterministically clears the high-volume, well-shaped PII for free — then the AI pass for names and context. Write both nodes' detection counts into the contract's run record; that's the evidence the masks actually ran on the copy you shipped.

Then place redaction correctly in the pipeline: **everything shared is built from masked text**. The team copy, the search index in lesson five — masked. The original stays in restricted storage with its provenance intact. Redaction that happens after sharing isn't redaction; it's an apology.

**Watch out:** a mask redacts what it *detects*, nothing more. Spot-check the Detections output on a sample of documents every time your document mix changes — a new contract template can introduce PII in places your configuration doesn't cover.

**Recap**

- Regex mask: shaped PII (emails, phones, IBANs, cards) — deterministic, free, local.
- AI mask: names and contextual PII — configurable mask text, sensitivity, and instructions.
- Layer regex → AI, keep detection counts as evidence, and share only masked text.
