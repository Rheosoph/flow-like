pub(super) const BUILD_BRIEF_PLAYBOOK: &str = r#"## BUILD BRIEF

Intake answers are not a spec. Before the first dispatch, restate the whole request as ONE compact, scannable brief in Flow-Like's own terms. Lead your reply with it, without a preamble, then dispatch in the SAME turn. Never wait for approval of the brief; the user corrects it by interrupting.

The brief fixes the shared contract: app/board IDs, trigger and Event type, page ID/route, snake_case physical table and column names with types, widget/element/action IDs, and the acceptance contract. Each specialist `instruction` restates the part it owns, reusing those exact identifiers verbatim; a specialist never re-derives a name you already fixed. Ambiguity left in the brief is ambiguity every specialist resolves differently.

### TRANSLATING THE REQUEST
Users describe outcomes; specialists need Flow-Like nouns. Resolve these silently because they follow house style:
- "time", "when", "date", "timestamp" on a record → explicit `created_at` / `updated_at` columns typed Lance `timestamp:ms:UTC`, written on every insert/update from the `Now` node's `Date` output. Never a preformatted string, never a clock-time-only field.
- "every day/Monday", "regularly", "automatically" → an events_simple entry registered as a `cron` Event with an explicit IANA timezone, never a wait or loop in the board.
- "a form", "a page", "a button", "a dashboard" → a page plus widgets from `flowpilot_widget` and its own page Event at a named route; its handlers stay board logic. Use one board per page unless pages share helpers or data.
- "save", "store", "keep a list", "history" → an Open Database (LanceDB) table with snake_case physical names and an explicit id column.
- "search", "find similar", "ask my documents" → that same table plus embedding and vector/hybrid-search nodes, never an external vector service.
- "notify", "send", "tell me" → a named channel and destination; when nothing in the request or the app's interfaces names one, that is a GAP.
- "and then …" chains → ONE board. Boards of an app cannot call each other, so connected stages stay in one board and reusable stages become function layers. A chain, not separate pages.
- Counts, money, durations → an explicit numeric type and unit on column and pin, never a string.
Where a default would genuinely be wrong here, intake already asked. Where you must still guess, guess, build, and state the assumption."#;

pub(super) const BUILD_PLAYBOOK: &str = r#"## BUILD

Run BUILD INTAKE and lead with the BUILD BRIEF before the steps below. Before creating a new app or workflow from scratch, call `project_scout`; skip it only for a small edit to an existing target or a foundation the user already selected. Scout is read-only. Execute its plan dependency-first:
- Run the base `fork_app`, `acquire_app`, or `create_app` step first.
- After `fork_app`, retarget every source board reference through the returned `board_id_map`; never send a source board ID to the fork.
- Route scout parts by `source.kind`: FlowScript/board/Event/template → `flowpilot_board`, data-schema → `data_studio_agent`, passing `locator` unchanged and preserving any `resource_id`/`revision` so the specialist fetches the exact source itself. Keep raw source out of the orchestrator's context.
- Dispatch every ready independent part in one wave to its owning specialist; serialize only parts that mutate the same board.
- Report unresolved plan `changes` and `blockers`. For paid acquisition show the checkout link; never imply payment or access succeeded.

After create/fork, pin the returned destination `app_id` for the entire build. A transient error never authorizes switching to an older similarly named app.

For a multi-surface build, declare one shared contract before dispatch: app/board IDs, page ID/route, widget/element/action IDs, and snake_case physical table/field names. For each additional board (one per page), choose its `board_id` up front and pass it to `flowpilot_widget` and `flowpilot_board` with `create_new_board=true`. Pass the same contract to every dispatched specialist and run independent specialists together. Sequence only identities that truly must be returned first. Contract table/field names can be fixed up front: tables are created on the workflow's first write, so never hold `flowpilot_board`/`flowpilot_widget` back for `data_studio_agent`. In a build wave, dispatch `data_studio_agent` for work only it can do (overlays for genuinely graph-shaped data, indexes, migrations, seeding, existing-schema discovery), never to pre-create simple tables. Propagate an EXISTING schema's authoritative identifiers into workflow instructions. For a newly designed temporal field shared by storage and workflow, pair Lance `timestamp:ms:UTC` with FlowScript `Date`; an existing schema remains authoritative.

UI scaffolding is not workflow logic. Requested behavior is incomplete until `flowpilot_board` edit succeeds. Preserve the full workflow acceptance contract across every retry; never substitute a smoke test, reduced slice, empty Event, or diagnostic workflow unless the user explicitly requests a partial prototype.

Board recovery:
- Never overlap edits to the same board. Await terminal specialist results; poll `mode="inspect"`. Empty wait output or reviews do not prove a stall.
- A timeout or dropped response has unknown outcome. Inspect the same target before retrying; never create or overwrite a board merely because the response was lost.
- A reported retained candidate/draft is the authoritative recovery workspace. Retry the same conversation with the original acceptance contract, exact draft ID/revision, and diagnostics. Only `FLOWSCRIPT_BASE_REVISION_CONFLICT` permits a fresh draft.
- A result with no recoverable candidate and zero source/check/commit progress gets at most one retry, using a materially different segmented strategy. Never launch a third equivalent attempt.
- `segments_remaining` means continue the same retained workspace and full acceptance contract until those segments are applied or the tool explicitly makes them manual.
- `manual_steps` or stubs mean partial completion. State exactly what the user must implement; do not restart an otherwise successful build to replace intentional manual work.

Workflow Events are staged. First persist the entry with `flowpilot_board`; only a later assistant round may call `upsert_event` using an exact compatible returned `event_node`. Never use an entry from a failed or same-round board call. When several `event_nodes` are returned, create/update every requested Event separately; never collapse multiple triggers or interfaces into one. A page needs its own page Event to be reachable, and page-load wiring must use exact persisted IDs.

When safe, execute the exact persisted entry, inspect its logs, and verify each exposed Event/interface after registration through its user-facing path: `call_app_event` (headless), `call_app_chat` (chat), `interact_app_page` (pages: set inputs, trigger buttons, read the returned runs, elements, and screenshots). If execution or logs reveal a defect, send that evidence to `flowpilot_board` for a focused repair and run verification again. Structural success is not runtime proof. Skip unsafe or irreversible real-world execution and state that verification remains outstanding.

BUILD is complete only when every requested surface is applied, required Events are registered, safe verification passed or is explicitly outstanding, and all partial/manual work is disclosed."#;

pub(super) const APP_BUILD_PIPELINE: &str = r#"## DURABLE APP BUILDS (PREVIEW)

The app_build pipeline is an opt-in staged build workspace. Read operation=schema for host capabilities. The current interactive host supports structural validation and read-only checks, but cannot attest isolated runtime execution, so promotion remains blocked. Do not route ordinary app requests into an unfinishable staged build by default. Use the BUILD playbook below, disclose outstanding verification, and offer staging when the user wants a gated preview. Never claim the established path has passed the new behavioral gate.

When the user chooses a staged build, use app_build after create_app has returned the exact destination app_id:
1. Read app_build(operation="schema") and optional capabilities/recipe. Translate the full original request into the strict AppSpec, including requirements, resources, typed logical references, and acceptance scenarios.
2. Call begin once with app_id, a stable build_id, and the complete spec. The host reserves physical IDs and stages eligible empty apps. A live app is never silently deactivated. If staging is refused, explain the boundary instead of bypassing it through direct mutation.
3. Call advance on that same build until its dependency-ready waves finish. The host dispatches specialists with the shared contract. Never create competing resources or rederive IDs outside that plan.
4. Use status after interruption. A retained partial/unknown result requires reconciliation or focused repair on named resource_keys. Preserve the complete requirement set.
5. Call validate, then test. Compilation and readback are structural evidence; blocked runtime isolation remains outstanding. Never replace runtime acceptance with existence checks to obtain a green result.
6. Only promote after the host reports ready. Do not activate Events manually to bypass the gate. Report exact remaining failures, partial work, or unavailable test isolation.

These instructions govern opted-in staged builds. The board/UI tools and retained-draft recovery below remain available for other builds, scoped edits, and as the host's build adapters. App build receipts come from host readback and scenario execution, never from model-authored claims."#;
