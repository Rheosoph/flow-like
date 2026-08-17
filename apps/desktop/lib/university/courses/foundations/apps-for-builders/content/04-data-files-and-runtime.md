Monday, 09:12. The Copilot answers its first live question with: "I couldn't find the refund policy." The flow is fine. The file is fine. The problem is *where the file lives*: you uploaded `refund-policy.pdf` to your own User Storage, and your teammate's run can't see your private files. Every resource in an App has a right home — and most incidents like this one are just a resource sitting in the wrong one.

## 1 · Files: shared or personal, never "somewhere"

Look at the sidebar in @StructuredData — under **Data** you'll find both file surfaces side by side: **Storage** and **User Storage**.

**Storage** holds files and folders shared across the App. Upload the refund policy and the support handbook here; every Flow run and every teammate resolves the same copy, and in an online App it syncs across devices. **User Storage** is private per user — right for an agent's personal draft notes, wrong for anything a teammate's run must read.

Give files stable paths and owners. A Flow that reads `/reference/refund-policy.pdf` should handle the file being missing or replaced, and your release check should confirm the target environment actually contains it. A temporary signed URL is not a file's durable identity.

## 2 · Rows and objects: Data Studio

Files answer "where's the document?" — but the Copilot also needs to *record every case it handles*. Case records are rows, and rows belong in **Data Studio**:

@StructuredData

This is the Copilot's data home base. The tiles count one ontology, six object types, two actions, one shared and one remote contract, and the semantic layer beneath them — "Customer Operations", six objects and six relationships over eleven tables. The tab row is the map: **Sources** (native tables), **Explore** (browse mapped objects), **Model** (ontologies and relationships), **Actions** (governed operations), **Sharing** (contracts between connected Apps), **Queries** (SQL with table, chart, graph, or JSON output).

The rule of thumb: use a **table** when the App needs durable rows with a schema; add an **ontology** on top when people or connected projects benefit from named objects, relationships, and governed actions. A Flow variable is neither — it's in-memory state for one run. `current_case_id` can live in a variable during an invocation; the case itself must land in a table, or it dies with the run.

## 3 · Configuration: values that vary by runner

Some values aren't data at all — they're configuration that differs per device or environment: local paths, endpoints, API keys. **Runtime Variables** keep those out of the Flow definition: the definition keeps the variable's name and type, while each device stores its own configured value locally.

Two switches matter:

- **Runtime Configured** — the runner supplies the value from outside the graph. A non-secret runtime value can travel along when an invocation executes remotely.
- **Secret** — the value is masked locally and **excluded from remote execution payloads**. It never leaves the device.

Enable both for a device-specific credential. Then follow the logic to its end: a remote, unattended run can't receive a secret that exists only on your laptop. Remote Events get their credentials from the deployment's server-side mechanism — and that's a feature, not a gap, because the alternative is your API key riding along inside run payloads.

> **Watch out:** runtime values live in local application storage — kept out of the synced App definition, yes, but that's separation, not encryption. Protect the OS account and the machine itself.

## Recap

- Shared files → Storage; personal files → User Storage; durable rows → Data Studio tables; one run's state → Flow variables.
- An ontology adds named objects, relationships, and governed actions on top of tables — add it when the domain earns it.
- Secret runtime values never travel to remote runs; unattended remote flows need server-side credentials.
