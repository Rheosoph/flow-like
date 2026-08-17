By Tuesday night you have 214 piles of markdown — thousands of pages of it. Wednesday morning, Legal asks its first real question: "Which contracts mention data-processing obligations?" A pile of text is not a filing system. This lesson turns each extracted contract into a structured record: tagged, sectioned, summarized, and traceable back to its source file.

> **Predict first:** Tagging 214 documents with keywords — does that need an LLM?

## 1 · Tag

It doesn't have to. **RAKE Keywords** and **YAKE Keywords** extract key phrases with deterministic algorithms — RAKE analyzes word frequency and co-occurrence, YAKE scores statistical features of the text itself. No model, no cost, and the same input produces the same keywords every run, which matters the day an auditor asks how a tag got there.

**AI Keywords** is the escalation: connect a model, set Max Keywords, and — the genuinely useful part — give it a context instruction like "focus on legal obligations." Reach for it when the tags you need live in *meaning* rather than surface phrasing. The docs' rule of thumb: deterministic when reproducibility and cost matter most; a model when the task depends on semantics.

## 2 · Slice

Keywords say what a contract mentions; sections say where. **Extract Content Sections** has a model read the pages and segment them into thematic sections — each with a title, a summary, its own keywords, and the page numbers where the theme appears. It tracks topics across non-contiguous pages, so a liability theme that surfaces on pages 3, 41, and 78 becomes one section, not three fragments. For an 80-page master services agreement, that's a generated table of what lives where.

## 3 · Compress

**Summarize Document** produces the executive view. You pick a detail level (Low, Medium, High), optionally a table of contents with page references, and a strategy — and the strategy choice is a real decision:

- **Refine** walks pages in order, carrying the accumulated summary forward. Slowest, most coherent — the documented fit for order-dependent text like legal documents.
- **MapReduce** summarizes chunks in parallel, then merges. Fast, but chunks can't see each other, so cross-chunk context gets lost.
- **Hierarchical** follows heading structure; **Hybrid** runs MapReduce speed with a Refine polish; **SlidingWindow** keeps constant memory for very long inputs.

A contract whose clause 14 leans on definitions from page 2 is the textbook case for Refine: order is information. There's also an optional Chain of Density post-pass that packs more facts into the same summary length — evaluate it with your configured model before trusting it in production.

## 4 · Record

Now assemble the deliverable: one structured record per contract. A workable schema: source path, detected file type, page count, keywords, summary, section list, processed-at timestamp. The non-negotiable field is **provenance** — every record points back at its file in storage. The original stays put (you learned why in lesson one: audit and reprocessing), and the record carries the reference.

Where do records live? In the app's database tables — lesson five wires the batch writes. Data Studio is where you model and explore what you've stored.

@DataStudioOverview

That's Data Studio's Overview tab in the same support app: tiles counting one ontology, six object types, two actions, and one shared contract; a "Your semantic layer" panel listing a Customer Operations ontology with six objects and six relationships; and task shortcuts like "Explore business objects" and "Shape the model." Your contract records can graduate into exactly that kind of modeled layer — the Data course owns that territory, along with the RAG and embedding internals that make records semantically searchable.

**Recap**

- RAKE/YAKE tag deterministically and free; AI Keywords when meaning beats surface terms.
- Extract Content Sections and Summarize Document add structure — pick Refine when order carries meaning.
- Every record keeps provenance to its source file; the original never gets replaced.
