Avery Morgan — Enterprise plan, one open ticket, last heard from twenty hours ago — types a question into your support portal: "Can I still get a refund on my annual plan?" You're building the app that answers her: the Customer Support Copilot. Before it replies, that one sentence will touch five different kinds of data, and each kind has a different home in Flow-Like. Pick the homes first; the nodes are the easy part.

> **Predict first:** while the copilot is mid-run, where does Avery's question text actually live — a file, a table row, or somewhere else entirely?

## 1 · Five homes for one question

**Typed flow values.** The moment the question arrives, it's a string on a pin, traveling between nodes. Pins carry strings, numbers, booleans, bytes, paths, structs, and JSON while a run executes — perfect for transforming and routing, gone when the run ends. That's the answer to the prediction: mid-run, Avery's question is a transient pin value. If a later event needs it, you persist it on purpose.

**App files.** Your team's refund policy is a document, not a record. Files live in app-aware storage: **Storage** for files the whole team shares (the refund policy), **User Storage** for files private to one user inside the app (a personal draft). A `Path` value points into one of those scopes — it's not a raw filesystem path.

**Native tables.** Avery herself — ID, plan, status, open tickets — is a row. Structured records that need schema, filters, upserts, and indexes belong in native tables, visible in Data Studio's **Sources** tab whether a person or a flow created them.

**Query sessions.** "How many refund tickets did each plan open last month?" is a question you answer by combining sources, not a new place to keep data. A DataFusion session registers files, tables, and external databases under names, runs SQL across them for one run, and vanishes. Durability stays with the sources — or with a result you explicitly write somewhere.

**Ontology metadata.** "This ticket belongs to Avery, and Jordan is handling it" is meaning layered over rows. An ontology maps existing tables to object types and relationships without copying a single row. One set of tables, many semantic views.

## 2 · See it in the product

Open any of your apps and go to **Data → Data Studio**. Two minutes of clicking beats any diagram: the tab bar walks the same boundaries you just read — **Sources** for native tables, **Queries** for SQL, **Model**, **Explore**, and **Actions** for the ontology, **Sharing** for semantic contracts.

@DataStudioOverview

That's the copilot's Data Studio on its **Overview** tab: summary cards counting 1 ontology, 6 object types, and 2 actions, a semantic layer spanning 11 tables, and a "Customer Operations" ontology card listing 6 objects and 6 relationships. Everything on that screen is something you'll have built by the end of this course.

## 3 · Classify before you build

Do the sort for the copilot right now, one line each: the incoming message text (pin value), a screenshot only its uploader may see (User Storage), ticket status and owner (table rows), the monthly summary (a session query over registered sources), "Ticket belongs to Customer" (an ontology link). For each, note who owns it, how long it must live, and how it gets deleted. If any answer feels fuzzy, the architecture isn't ready — and that's much cheaper to discover now than after an import runs twice.

**Watch out:** a DataFusion session feels like a database, but it isn't storage. Registered tables and query results disappear with the run unless you write them somewhere durable.

## Recap

- Pins move values during a run; nothing survives unless you persist it deliberately.
- Files go to Storage (shared) or User Storage (private); records go to native tables; sessions only combine sources for one run.
- An ontology adds meaning over tables without copying rows.
