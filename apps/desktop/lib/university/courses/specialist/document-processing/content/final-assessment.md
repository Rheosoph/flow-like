Friday, 9:00 a.m. The compliance pipeline goes to its go-live review. You're the one who built it, so you're the one answering. Everything you need is in the artifacts below — the challenges ask you to make the calls.

## The system under review

The compliance inbox: contracts arrive as files in the app's storage under contracts/incoming/. A nightly flow lists the folder, routes each file by type, extracts text, builds a structured record (keywords, sections, summary), masks PII, then delivers three writes — the record to a database table, a redacted copy to a shared prefix, and the search index. Originals remain in restricted storage.

## Legal's requirements

- **R1** — New contracts must be searchable by the team within 24 hours of arrival.
- **R2** — Emails, IBANs, and signer names must never appear in any shared copy or search result.
- **R3** — For any record, it must be possible to reproduce how it was built, and to show evidence that masking ran on the shipped copy.
- **R4** — Originals are retained unmodified for seven years.

## IT's constraints

- **C1** — The run executes unattended in a nightly window.
- **C2** — Model spend is budgeted; per-page AI calls need justification.
- **C3** — Models are configured per profile, not hard-coded into boards.

## Pilot findings

- **S1** — Of 214 files in the pilot batch, 24 produced empty text from the deterministic extractor. All 24 came from the office scanner.
- **S2** — One Debug log line from the pilot contains a full contract paragraph.
- **S3** — Thursday's run failed at document 178. The on-call engineer couldn't say whether rerunning was safe, so nothing ran until morning.
- **S4** — A teammate proposes indexing the unredacted text "for better search recall," arguing the team only ever sees snippets.
- **S5** — Monday's inbox will add roughly 30 new contracts and occasionally an updated amendment to an existing one.

## Your role

Each challenge below hands you one review question. Decide as the person who has to operate this pipeline — and defend the answer with what you built across the last five lessons.
