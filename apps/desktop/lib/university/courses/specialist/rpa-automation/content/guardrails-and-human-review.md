The flow now reads all forty statuses in under two minutes, and the spreadsheet fills itself. Then Tuesday's report shows three purchase orders marked "Delayed — action required," and next to each, the portal offers a friendly Acknowledge button. The flow could click it. It's one more node. Should it?

> **Predict first:** what's the actual difference between reading a status and acknowledging a delay?

## Reads and writes are different animals

Reading a status changes nothing in the world; if the extraction is wrong, you re-run it. Acknowledging a delay is a statement *to your supplier* — it can carry contractual weight, and no re-run unsays it. That line — reversible reads, consequential writes — is where automation authority should stop by default. And in Kestrel's case it isn't even your call: the written authorization covers automated *read-only* status checks. Full stop. Automate the reading; gate the writing.

## Scope is a design input

The minimal-scope rule you applied to OS permissions in the desktop lesson applies to agreements too: automate only what you're authorized to automate, respect the portal's terms, and treat scope as part of the design rather than paperwork to check later. Keep credentials out of the flow's visible configuration as well — the API Integrations course covers the secret-handling patterns that apply here.

## The human checkpoint

@FlowLikeStudio

The board above isn't a Kestrel flow — it's a support pipeline — but its silhouette is the lesson: three labeled stages ("1 · Listen for requests", "2 · Draft with AI", "3 · Approve and send"), and between the drafting layer and Send Reply sits a **Human Review** placeholder carrying the note "Prototype a future review step before implementing its internals." The checkpoint is part of the flow's shape before its internals even exist. Give the Kestrel flow the same silhouette: gather → propose → a person approves → only then act. For the three delayed POs, the flow's job ends at a tidy summary with evidence attached. Dana's judgment — not a node — clicks Acknowledge.

## What leaves the screen

Screen captures and extracted UI content can contain personal, confidential, or regulated data; Kestrel's order pages carry contact names and delivery addresses. If a capture goes to a configured model or any external service, the provider, connection, and organizational policies govern where that content is processed. So minimize what you capture — **Screenshot Element** for a single browser element, **Screenshot Region** instead of a full desktop capture — and redact sensitive values before anything is stored or sent.

## LLM assistance: propose, then validate

The catalog's LLM automation nodes come in three groups: vision (**LLM Observe Screen**, **LLM Find Element**, **LLM Extract From Screen**), planning (**LLM Plan Actions**, **LLM Suggest Next Step**), and healing (**LLM Heal Selector**, **LLM Heal Template**, **LLM Diagnose & Heal**). Reach for them when deterministic targeting runs out of signal — and treat every output as a proposal. Validate the element it picked, bound the fallback, and keep consequential actions behind the checkpoint you just designed. A model that's been right twice has earned exactly nothing unattended.

## Recap

- Automate the reading; every action with business meaning waits for a person.
- Captures can carry personal data — minimize the region, redact, and know your policies before content leaves the machine.
- LLM nodes propose; the flow — and the human — dispose.
