Avery Morgan is on the phone again. The agent taking the call needs one thing, fast: which colleague handled her last ticket, and how it ended. That answer lives across three tables — `customers`, `tickets`, `agents` — and nobody wants to hand-write the join mid-call. This is what Flow-Like's ontology layer is for: it turns rows into objects you can click through — Customer → tickets → assigned agent — without moving a single row.

@OntologyOverview

The diagram shows the idea with a logistics example: existing tables on the left (People, Vehicles, Shipments), ontology metadata in the middle — object types (Person, Vehicle, Shipment), link types (drives, carries, ships_to), and governed actions (Approve, Enrich, Dispatch) — and the payoff on the right: search objects, traverse relationships, run governed actions. The callout along the bottom is the contract: metadata only, no source rows are copied when the ontology is created.

## 1 · The overlay

Three definitions carry the whole layer:

- An **object type** maps one table to a business entity: a stable API name, a unique-ID column, a display property, and projected properties.
- A **link type** maps relationship rows or columns between a source and a target object type — that's what powers neighbors, traversal, path finding, and subgraphs.
- An **overlay** stores all of it — objects, links, styling, views, actions — next to the tables it references. The tables stay authoritative, and several overlays can interpret the same data for different audiences.

In Data Studio: **Model** defines mappings, **Explore** browses objects, **Actions** governs operations, **Queries** can render results as relationship graphs, and **Sharing** exposes semantic contracts to other projects.

## 2 · Model Customer Operations

Remember the "Customer Operations" card from lesson 1?

@DataStudioOverview

That's the destination, on Data Studio's Overview tab: 1 ontology, 6 object types, 2 actions, a semantic layer over 11 tables. Its heart is built from the three tables in our phone call:

1. Map `customers.customer_id` to a **Customer** object with `name` as display property.
2. Map `tickets.ticket_id` to a **Ticket** with status, priority, and created time.
3. Map `agents.agent_id` to an **Agent**.
4. Link Customer → Ticket through the ticket's `customer_id`.
5. Link Ticket → Agent through `assigned_agent_id`.
6. Open Explore, find Avery, and traverse to her tickets and their agents — the mid-call question, answered by clicking.
7. Define an "assign ticket" governed action instead of letting every interface mutate the table directly.

Before declaring an identity column, verify it's actually unique — duplicate or null IDs make traversal ambiguous, and one customer's page starts showing someone else's tickets. Verify link columns reference real targets, and decide how a missing target should appear.

Flows can do everything the UI can: catalog nodes create, open, and drop overlays, upsert graph nodes and edges, and query by neighbors, paths, Cypher, or SQL. Use the UI for reviewed modeling, nodes for automated lifecycles — the same identity and access rules apply either way.

## 3 · Evolve without breaking it

Renaming a source column or changing its meaning silently invalidates object properties and links — review overlays before schema mutation, right next to lesson 3's saved-query check. Dropping an overlay deletes definitions, not tables; dropping a table is the destructive act. And sharing an ontology exposes a governed contract, not the rows: consumers can't write producer data, and mapping a table into an overlay never grants anyone new access to it.

**Watch out:** if you catch yourself copying rows into a "graph table" so the graph has data — stop. An overlay maps tables in place. Copies drift, and deletion in two places reliably happens in neither.

## Recap

- An ontology is metadata over authoritative tables: object types, link types, actions — zero copied rows.
- Identity first: unique ID columns and validated link targets are what make traversal trustworthy.
- Overlay lifecycle is not table lifecycle, and overlay visibility is not table authorization.
