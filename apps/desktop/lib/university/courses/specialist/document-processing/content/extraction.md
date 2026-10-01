Use **Extract Document** with a typed Path to each fixture file. It returns pages with content and page identity. Print a page count or short synthetic excerpt, then inspect the full output in your practice run.

1. Extract `digital-agreement.pdf`. Check the agreement ID, renewal period, fee and email against `expected.json`.
2. Extract `scanned-agreement.pdf`. Inspect the pages rather than treating a green run as proof of useful text.
3. If the scan has no useful text, connect its original file Path to **AI Extract Document**'s File input and choose a configured vision-capable model. The File input takes a Path, so keep the extracted Pages output separate.
4. Extract `mixed-agreement.pdf`. Inspect both pages separately. A non-empty first page does not prove the second page was read.
5. Compare every result with the known fixture facts. Route missing or altered facts to review.

Use **Pages to Markdown** when you need a combined string, keeping page markers. For a targeted retry of the mixed file's second page, use **Split PDF** with Start Page and End Page both set to `2`, plus a scratch Output Path. Pass its Result Path to AI Extract Document. Record that page 1 of this split file came from page 2 of the original source.

Model transcription may change numbers or miss text. Empty output is one quality signal; partial text, broken tables and missing pages also matter. Record which extraction method produced each page and which checks passed.

If your profile has no vision model, retain the scan as a review item. Do not record it as successfully extracted. Complete the digital path, then return when the prerequisite is available.
