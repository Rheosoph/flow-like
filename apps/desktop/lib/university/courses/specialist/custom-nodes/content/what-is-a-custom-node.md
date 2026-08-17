Type "mail" into a board's node search and Flow-Like instantly offers Send Email, Parse Mailbox, Watch Inbox. Now type "normalize". Nothing. Acme's support team has a problem the catalog can't solve: customer messages arrive looking like `"   URGENT!!   My PRINTER Is On FIRE   "`, and they want the text cleaned up before an AI drafts a reply. That's the gap you close in this course. You'll build `normalize_text`, a real WebAssembly node, and by the last lesson it shows up in that same search box as if it had always been there.

@NodeCatalog

This is where your node will land. In the screenshot, a support board is mid-build — Incoming Support Request feeds Prepare Support Reply, then Human Review, then Send Reply — and the builder has typed "mail" into the node search. The catalog answers with built-in mail nodes: Add Mail Attachment, Copy Mail Message, Send Email, Parse Mailbox, Watch Inbox. Nodes from installed packages appear in this exact list, side by side with the built-ins.

> **Predict first:** to get a node into that list, do you have to modify Flow-Like itself?

## Two ways in

You don't — and that's the whole point. Flow-Like has two extension paths, and picking between them is a distribution decision, not a matter of taste.

A **native node** is Rust code compiled into Flow-Like's own catalog. Write one when you're contributing generally useful behavior to the Flow-Like repository and can live with its release cycle. Acme's normalizer doesn't qualify: it's team-specific, and waiting for a product release to fix a whitespace bug would be absurd.

A **WASM node** ships as an independent package. You build it from a maintained template, version it on your own schedule, and Flow-Like loads it into a Wasmtime sandbox with memory, time, and capability boundaries. Private distribution, no fork, no product release. This is your path — with one honest caveat: the sandbox limits what damage code can do, it doesn't make unknown code trustworthy. Treat every package you install the way you'd treat a browser extension.

## One binary, two shapes

@WasmRuntimeModels

The diagram shows what happens when Flow-Like loads your `node.wasm`: the runtime checks the binary format and adapts. A Component Model header routes it through **WIT and the canonical ABI** — typed strings, lists, and interface calls. Anything else is treated as a **core module** — raw exports with JSON crossing linear memory. Both shapes flow into the same capability gate on the right, where declared permissions become runtime limits and host access.

You'll use the Component Model: it's the preferred path and powers the maintained TypeScript, Rust, Go, Python, and other component templates. Core modules exist for compatible toolchains such as AssemblyScript. Either way, start from a maintained template — an arbitrary `.wasm` file won't implement the exports Flow-Like expects.

## One contract, two phases

Every node lives a double life. Its **definition** tells the editor everything it needs before the node ever runs: an internal name, a friendly label, a category, pins with types and defaults, and any permissions. Its **run** function does the actual work at execution time: read inputs, produce outputs, activate execution pins.

Pins come in two roles. *Data pins* carry typed values — strings, integers, booleans, bytes, JSON. *Execution pins* decide which downstream path runs. A pure calculation like text normalization could get by with data pins alone; a node with side effects or ordering needs takes an execution input and activates an execution output. You'll give `normalize_text` execution pins so the support board can sequence it explicitly between "message arrives" and "draft the reply".

> **Watch out:** the internal node name and pin names are stored inside every saved board. Rename `normalize_text` to `clean_text`, or its `result` pin to `output`, after people depend on it, and their boards break — even if the friendly labels never changed. Internal names are API.

## Recap

- A WASM package is the independently versioned, privately distributable extension path; native Rust nodes are for the product's own catalog.
- The definition drives the editor; `run` drives execution — one contract, two phases.
- Internal node and pin names outlive you. Choose them like you'd choose a database column name.
