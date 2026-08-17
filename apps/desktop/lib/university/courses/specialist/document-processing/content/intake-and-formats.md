It's Monday, and the compliance inbox holds 214 files. Signed contracts from the e-signing service, NDAs the office scanner produced, a handful of DOCX drafts someone exported "just in case." Legal wants all of it searchable and redacted by Friday. Before a single node runs, you need to answer one question per file: can a computer read this, or does it just *look* readable?

> **Predict first:** Open two of the PDFs side by side — one exported by the signing service, one from the office scanner. In which one can you select and copy a sentence?

Try it in any viewer. The export lets you select text; the scan doesn't, because a scan is a photograph of a page wearing a `.pdf` badge. That one distinction decides most of your pipeline, and this lesson gives you the vocabulary for it.

## 1 · Where the paper lands

Every Flow-Like app has a Storage page — the shared drive your flows can actually reach.

@AppStorage

That's the Storage page of a support app: two folders (archived-tickets and customer-briefs) plus three loose files — brand-voice.md, refund-policy.md, and a 2.8 MB support-playbook.pdf — with New Folder, Upload Files, and Upload Folder buttons at the top right. Open any app of yours, click Storage in the sidebar, and drag a PDF in. That's your quick win: the file is now inside the pipeline's reach, and every flow in the app can read it.

For the compliance inbox, create a contracts/ folder and drop this week's arrivals there. Folders are your first structure — an incoming/ prefix now becomes a clean processed/ and review/ split in lesson five.

## 2 · Know what you're holding

Formats sort into families, and each family has its own reader:

- **Digital PDFs** carry a text layer — the characters live in the file. Deterministic extraction just works.
- **Scanned PDFs** carry pictures of text. Nothing is extractable until something *looks* at the page — that's the vision path in lesson two.
- **DOCX and PPTX** are structured office formats with native extraction nodes.
- **Spreadsheets and CSV** are data, not prose — they get cell and table readers, not text readers.
- **Images and HTML** each have converters of their own.

The selection test you just ran is the honest check for the first two: selectable text means a text layer; a stubborn page means a scan.

One habit that saves real debugging time: treat the extension as a claim, not a fact. A CSV renamed to `.pdf` happily wears the wrong badge, and the docs are blunt about it — a file extension alone is not a sufficient trust boundary. Validate the type before you route the file.

## 3 · Inventory before you process

Monday's first deliverable isn't extraction — it's a count. The wiring is short: Storage Dir gives your flow a path into the app's storage, List Paths enumerates every file under your contracts/ prefix, and Head reads a file's metadata, like its size, without downloading it. That's enough to answer: 214 files, how many per family, which folder each sits in.

Here's the map for everything that comes after.

@DocumentProcessingOverview

Read its lanes left to right: documents come **in** (PDF, DOCX, Excel and CSV, image, HTML — from local or app storage), get **prepared** (read, render, stream; split, batch, convert; preserving source metadata), get **understood** (markdown-aware text, tables as rows and cells, typed schema extraction, an optional configured AI model), and get **delivered** as useful outputs — database records, a search index, a report or template, flowing out through API, storage, or notification. Five lessons from now, you'll have built exactly that for the compliance inbox.

**Recap**

- Storage is where flows and files meet; folders are your first pipeline structure.
- Digital PDF ≠ scanned PDF: the text layer decides the extraction path.
- Inventory with Storage Dir + List Paths — and never trust an extension.
