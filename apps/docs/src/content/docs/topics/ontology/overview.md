---
title: Ontology & Knowledge Graph
description: Turn your data tables into a semantic layer of object types, relationships, and governed actions, then explore and act on them as a graph.
sidebar:
  order: 1
---

An **ontology** is a semantic layer you place over the database tables you already have. Instead of thinking in rows and columns, you describe your data as **object types** (Person, Vehicle, Shipment…) connected by **link types** (drives, ships_to, works_at…). Flow-Like then lets you search, traverse, visualize, and run governed actions on those objects — a Palantir-Foundry-style knowledge graph — without moving or copying any data.

:::tip[Nothing is duplicated]
An ontology is **metadata only**. It maps your existing tables to object and link types; the tables stay exactly as they are and keep working as normal databases. You can define many ontologies over the same tables.
:::

## Where it lives

Everything is in **Data Studio**, reached from an app's data/explore area. Data Studio has four tabs:

| Tab | What it's for |
|-----|---------------|
| **Model** | Create, rename, and delete ontologies; browse object types |
| **Explore** | The graph visualizer — search objects, expand connections, find paths, run analytics |
| **Actions** | Define governed operations that run on an object type |
| **Sharing** | Expose your ontology to connected projects, and install ontologies from others |

## The building blocks

### Object types

An **object type** maps one table to a kind of entity. Each object type has:

- **Label** — the name used in queries (e.g. `Person`). Must be a valid identifier (letters, digits, underscores).
- **API name** — a stable identifier used by generated nodes and the API. Derived from the label by default; you can edit it.
- **ID column** — the column that uniquely identifies each object.
- **Display column** — an optional human-readable caption (e.g. a name or title).
- **Property columns** — the columns projected onto each object.

### Link types

A **link type** maps a table to a relationship between two object types — a source object, a target object, and the columns that hold their IDs. Link types power traversal ("who does this person know?"), path finding, and the graph view.

### Graph overlay

An **overlay** is the whole ontology: its object types, link types, styling, object views, and actions, stored as a single metadata record inside the same database as the tables it references. Because it lives next to the data, scope is automatic — a **project** overlay lives in the project database, a **user** overlay in the user database.

## Setting one up

Use **Model → Create ontology** to open the setup wizard:

1. **Sources** — pick the tables to include.
2. **Objects** — for each table, confirm the label, API name, ID column, display column, and properties. Duplicate labels or API names are flagged inline.
3. **Relationships** — Flow-Like infers link types from foreign-key-like columns. Review them, rename or remove any, and keep your edits as you navigate between steps.
4. **Publish** — the draft is validated against the live database (tables and columns must exist, labels must be unique and queryable). Anything wrong is shown per mapping before the ontology is saved.

Once saved, the ontology appears in **Model**, and — if bindings are enabled — its objects become available as board nodes (see below).

## Exploring the graph

The **Explore** tab renders your objects and links with a WebGL graph:

- **Search** matches loaded nodes first, then falls back to a full-graph search.
- **Click** a node to open its inspector; **shift-click** (or the inspector's **Expand**) pulls in its neighbors. Depth-2 expansion is available now that multi-hop traversal is honored end to end.
- **Find paths** — from a node's inspector, choose "Find path from here", then click a second node. Flow-Like returns the shortest connections (with alternative routes) and highlights them. This is the "how are these two things connected?" question.
- **Analytics** surfaces object counts, connected components, and the most connected / most central objects.
- **Object views** — the inspector shows a title property and prominent properties first, and offers any actions defined for that object type.

:::note[Performance]
Queries are bounded server-side: a shared concurrency limit, a per-query row cap, and a Cypher pre-flight that rejects unbounded variable-length paths. Use the limit selector and targeted expansion for large graphs rather than loading everything at once.
:::

## Actions on objects

An **action** is a governed operation that runs on an object type — "Approve order", "Enrich contact", "Dispatch vehicle". Define one in the **Actions** tab:

- **Object type** the action applies to, and an **implementation board** plus its **start node**.
- **Board version** — pin a specific published version for a reproducible action, or keep the current draft (it is published automatically when you save). Pinning to an immutable version is what makes an action safe to expose and re-run.
- **Parameters** — inferred from the start node's `parameters` pin schema, and validated on every invocation.
- **Enabled** and **Allow bulk** (up to 100 objects) toggles.

Under the hood, saving an action materializes a protected, version-pinned internal event and stores a hash of the whole contract. At invoke time the server re-checks that hash, so edits to ontology metadata can never widen what an action is allowed to read or execute.

## Using an ontology in boards

When **Generate board bindings** is on, the ontology contributes nodes to the app's node catalog so your flows can work with objects directly:

- **Query Ontology Objects** — fetch objects of a given type.
- Action request nodes — assemble a governed action invocation from object references.

There are also general graph nodes for advanced use — **Cypher Query**, **Graph Neighbors**, **Graph Subgraph**, **Find Paths**, and **Graph Analytics** — under *Data → Database → Graph*.

## Next

- [Shared & Remote Ontologies](/topics/ontology/remote/) — expose an ontology to connected projects and install ontologies published by others.
