import type {
	BuildFlowPilotE2EPromptOptions,
	BuiltFlowPilotE2EPrompt,
	FlowPilotE2ECaseDefinition,
	FlowPilotE2ECaseId,
	FlowPilotE2ECaseRequirements,
	FlowPilotE2EModelConfig,
	FlowPilotE2ERunOptions,
} from "./types";

export const FLOWPILOT_E2E_DEFAULT_MODEL: Readonly<FlowPilotE2EModelConfig> =
	Object.freeze({
		provider: "codex",
		model: "gpt-5.6-terra",
		reasoningEffort: "high",
	});

export const DEFAULT_MIN_FLOWSCRIPT_NON_WHITESPACE_CHARS = 700;
export const DEFAULT_MAX_FLOWSCRIPT_NON_WHITESPACE_CHARS = 16_000;

function requirements(
	overrides: Partial<FlowPilotE2ECaseRequirements> = {},
): FlowPilotE2ECaseRequirements {
	return {
		minFlowScriptNonWhitespaceChars:
			DEFAULT_MIN_FLOWSCRIPT_NON_WHITESPACE_CHARS,
		maxFlowScriptNonWhitespaceChars:
			DEFAULT_MAX_FLOWSCRIPT_NON_WHITESPACE_CHARS,
		minBoards: 1,
		minTotalNodes: 4,
		minPages: 1,
		minWidgets: 0,
		minTables: 1,
		minEvents: 1,
		requireAuthoredFlowScript: true,
		requireAuthoredLintDiagnostics: true,
		requireCanonicalFlowScript: true,
		requireLintDiagnostics: true,
		requireAuthoritativeReconcile: true,
		requireSuccessfulCompilerReceipt: true,
		validateReferenceIntegrity: true,
		requiredSemanticTableAliases: [],
		requiredIdReferences: [],
		requiredNodeCapabilities: [],
		...overrides,
	};
}

export const FLOWPILOT_APP_CREATION_CASES = [
	{
		id: "simple-agent",
		title: "Simple research and file-library agent",
		description:
			"Exercises web research, on-demand upload extraction, vector ingestion and retrieval in one chat-event workflow — no UI or Data Studio scaffolding.",
		appName: "Simple Agent",
		smoke: true,
		prompt: `Build a research agent that lives entirely in chat: implement everything behind exactly ONE Chat Event entry (eventsChat) and answer through the chat response. Do not add eventsSimple or eventsGeneric entries — branch on the incoming message inside the workflow, using functions for the sub-behaviors. Do not build any pages or widgets — the chat surface is the whole UI — The database tables are created automatically by the workflow's first write.

The agent must:
1. Search the internet and perform deeper research through https://search.flow-like.com (SearXNG), citing source URLs in the chat reply.
2. Read user-uploaded chat attachments on demand. Do not extract every file eagerly, and handle an unknown or missing filename safely in the reply.
3. When asked to store a file for later use: extract its text, split the text into chunks with a dedicated text-chunking node, embed the chunks, and persist file metadata plus searchable chunks with database writes into tables named exactly "Library Files" and "Library Chunks".
4. When asked about stored knowledge, search the file library semantically and answer with the matching chunks and their source files.

Reply with clear messages for empty results and errors. Keep everything on one coherent workflow board.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 1_000,
			minTotalNodes: 10,
			minPages: 0,
			minWidgets: 0,
			minTables: 0,
			minEvents: 1,
			requiredNodeCapabilities: [
				{ alias: "chat_event", anyOf: ["events_chat"] },
				{
					alias: "web_request",
					anyOf: ["http_fetch", "http_make_request", "streaming_http_fetch"],
				},
				{
					alias: "attachment_extraction",
					anyOf: [
						"ai_gen_llm_history_extract_attachments",
						"ai_processing_extract_document",
						"ai_processing_extract_document_ai",
						"ai_processing_extract_documents",
						"ai_processing_extract_documents_ai",
					],
				},
				{
					alias: "text_chunking",
					anyOf: [
						"chunk_text",
						"chunk_text_char",
						"ai_generative_llm_chunk_from_string",
					],
				},
				{ alias: "document_embedding", anyOf: ["embed_document"] },
				{
					alias: "database_write",
					anyOf: [
						"insert_local_db",
						"upsert_local_db",
						"batch_insert_local_db",
						"batch_upsert_local_db",
					],
				},
				{
					alias: "semantic_search",
					anyOf: ["vector_search_local_db", "hybrid_search_local_db"],
				},
			],
		}),
	},
	{
		id: "forum",
		title: "Community forum",
		description:
			"Exercises relational CRUD, reusable list widgets and widget-action events.",
		appName: "Pocket Forum",
		smoke: true,
		prompt: `Build a compact community forum. Create a page named "Forum" with a new-thread form, a thread list, a selected-thread view, and a reply form. Repeated threads must use one widget named "Thread Card".

Persist threads in a table named exactly "Forum Threads" and replies in "Forum Posts". The workflow must list threads newest-first by READING the persisted rows back from the table with a database read/filter node (never from in-memory state), create a thread with validation, open one thread with its replies, and add a reply. Wire the real "Forum" page and "Thread Card" widget ids into the workflow, and refresh the relevant view after each write. Include useful empty and validation states. Register EVERY workflow entry as an app event before reporting completion — the page-load entry AND each user-action entry (new thread, reply, open thread), at least two registrations in total.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 800,
			minTotalNodes: 8,
			minWidgets: 1,
			minTables: 2,
			minEvents: 2,
			requiredSemanticTableAliases: ["forum_threads", "forum_posts"],
			requiredIdReferences: [
				{ entity: "table", alias: "forum_threads", source: "canonical" },
				{ entity: "table", alias: "forum_posts", source: "canonical" },
				{ entity: "widget", alias: "thread_card", source: "canonical" },
			],
			requiredNodeCapabilities: [
				{
					alias: "database_read",
					anyOf: [
						"filter_local_db",
						"list_local_db",
						"fts_search_local_db",
						"vector_search_local_db",
						"hybrid_search_local_db",
						"df_sql_query",
					],
				},
				{
					alias: "database_write",
					anyOf: [
						"insert_local_db",
						"upsert_local_db",
						"batch_insert_local_db",
						"batch_upsert_local_db",
					],
				},
				{ alias: "widget_render", anyOf: ["a2ui_instantiate_widget"] },
			],
		}),
	},
	{
		id: "ops-dashboard",
		title: "Service operations dashboard",
		description:
			"Exercises a page-load pipeline, analytical table reads and concrete dashboard element updates.",
		appName: "Ops Pulse",
		smoke: true,
		prompt: `Build an operations dashboard page named "Ops Dashboard" for an on-call lead. It needs KPI cards for healthy services, open incidents, and mean time to resolution; a severity breakdown chart; and a recent-incidents table with a service filter.

Use tables named exactly "Services", "Incidents", and "Metric Snapshots". The page-load workflow must READ the persisted rows back from those tables with database read/filter nodes (never from in-memory state), calculate the KPIs, populate the chart/table, and handle an empty dataset. The filter must rerun the view with the selected service. Use the persisted page/component ids rather than guessed ids. This is a one-off dashboard, so do not create a reusable widget merely to satisfy the test.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 700,
			minTotalNodes: 8,
			minTables: 3,
			minEvents: 1,
			requiredSemanticTableAliases: [
				"services",
				"incidents",
				"metric_snapshots",
			],
			requiredIdReferences: [
				{ entity: "page", alias: "ops_dashboard", source: "canonical" },
				{ entity: "table", alias: "incidents", source: "canonical" },
			],
			requiredNodeCapabilities: [
				{
					alias: "database_read",
					anyOf: [
						"filter_local_db",
						"list_local_db",
						"fts_search_local_db",
						"vector_search_local_db",
						"hybrid_search_local_db",
						"df_sql_query",
					],
				},
				{
					alias: "dashboard_update",
					anyOf: [
						"a2ui_push_csv_to_chart",
						"a2ui_update_table",
						"a2ui_set_element_text",
					],
				},
			],
		}),
	},
	{
		id: "expense-approval",
		title: "Expense approval queue",
		description:
			"Exercises state transitions, audit persistence and multiple widget actions.",
		appName: "Expense Desk",
		smoke: false,
		prompt: `Build an expense-approval app with a page named "Expense Queue". Show pending requests as instances of a reusable widget named "Expense Row" with Approve and Reject actions, plus status/amount filters and an audit panel.

Use tables named exactly "Expense Requests" and "Expense Audit". The workflow must load pending expenses, approve or reject exactly one request, require a rejection reason, prevent a request from being decided twice, append an audit record, and refresh the queue. Wire both real widget action ids and persisted table ids into the workflow. Register the workflow's non-widget entries (the page-load entry and each simple/generic entry such as filter or audit refreshes) as app events before reporting completion — at least three registrations in total. Widget-action handlers are bound at widget instantiation via fnRefs; they are not registrable app events, so leave them out of registration.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 850,
			minTotalNodes: 9,
			minWidgets: 1,
			minTables: 2,
			minEvents: 3,
			requiredSemanticTableAliases: ["expense_requests", "expense_audit"],
			requiredIdReferences: [
				{ entity: "table", alias: "expense_requests", source: "canonical" },
				{ entity: "table", alias: "expense_audit", source: "canonical" },
				{ entity: "widget", alias: "expense_row", source: "canonical" },
				{ entity: "widget_action", alias: "approve", source: "canonical" },
				{ entity: "widget_action", alias: "reject", source: "canonical" },
			],
		}),
	},
	{
		id: "rss-digest",
		title: "RSS research digest",
		description:
			"Exercises HTTP ingestion, deduplication, scheduled processing and reusable article presentation.",
		appName: "Signal Digest",
		smoke: false,
		prompt: `Build an RSS digest app with a page named "Daily Digest" and a repeated widget named "Article Card". Users can add or disable a feed and view the latest digest grouped by topic.

Use tables named exactly "RSS Feeds", "RSS Articles", and "RSS Digests". A scheduled/manual refresh workflow must fetch enabled feeds, parse entries, deduplicate by canonical URL, persist new articles, create a concise digest, and render article cards. One broken feed must not discard successful feeds. Wire the real table, page, and widget ids.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 800,
			minTotalNodes: 9,
			minWidgets: 1,
			minTables: 3,
			minEvents: 2,
			requiredSemanticTableAliases: [
				"rss_feeds",
				"rss_articles",
				"rss_digests",
			],
			requiredIdReferences: [
				{ entity: "table", alias: "rss_feeds", source: "canonical" },
				{ entity: "widget", alias: "article_card", source: "canonical" },
			],
		}),
	},
	{
		id: "incident-console",
		title: "Incident response console",
		description:
			"Exercises event-driven operational state, idempotent actions and timeline updates.",
		appName: "Incident Console",
		smoke: false,
		prompt: `Build an incident-response console page named "Incident Console". Render active incidents with a reusable widget named "Incident Row" that has Acknowledge and Resolve actions, and show the selected incident's timeline.

Use tables named exactly "Incidents" and "Incident Updates". Support creating an incident from an incoming/manual event, loading active incidents, acknowledging once, resolving with a summary, and appending every transition to the timeline. Reject invalid state transitions and make repeated action delivery idempotent. Wire the real page, table, widget, and widget-action ids. Register the workflow's non-widget entries (the page-load entry and each simple/generic entry such as filter or audit refreshes) as app events before reporting completion — at least three registrations in total. Widget-action handlers are bound at widget instantiation via fnRefs; they are not registrable app events, so leave them out of registration.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 850,
			minTotalNodes: 10,
			minWidgets: 1,
			minTables: 2,
			minEvents: 3,
			requiredSemanticTableAliases: ["incidents", "incident_updates"],
			requiredIdReferences: [
				{ entity: "table", alias: "incidents", source: "canonical" },
				{ entity: "widget", alias: "incident_row", source: "canonical" },
				{
					entity: "widget_action",
					alias: "acknowledge",
					source: "canonical",
				},
				{ entity: "widget_action", alias: "resolve", source: "canonical" },
			],
		}),
	},
	{
		id: "mail-approval",
		title: "Email automation with mail-loop human approval",
		description:
			"Exercises IMAP/SMTP round-trips and a human-in-the-loop approval cycle carried entirely over email — no pages or widgets.",
		appName: "Mail Copilot",
		smoke: false,
		prompt: `Build an email automation whose human-in-the-loop approval happens entirely over email — this app has NO pages and NO widgets; do not create any UI.

Triage entry (manual/scheduled): connect to the mailbox over IMAP, fetch unseen inbox mail, extract each message's content, draft a reply, and send that draft over SMTP to the approver's address for review — tag the approval mail's subject with a stable draft id so the response can be matched later. Persist each draft (draft id, original sender, subject, draft body, status "awaiting_approval") with database writes into a table named exactly "Mail Drafts", and mark the source mail as seen.

Approval entry (separate manual/scheduled entry): fetch the approver's responses over IMAP and match each response to its pending draft by the tagged draft id, reading the draft back from the table with a database read/filter node. If the response approves (e.g. contains "ok"), send the approved reply over SMTP to the ORIGINAL sender and update the draft row to status "sent". Otherwise treat the response body as improvement feedback: revise the draft with it, send the revised draft back to the approver for another round, and update the row (status stays "awaiting_approval", revision incremented). Never send anything to the original sender without an approval.

Leave every server/credential/approver-address input empty ("") — the user fills them in later; never invent hosts, accounts, or passwords. Handle an empty inbox and an unmatched approval response safely. Register BOTH workflow entries as app events.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 900,
			minTotalNodes: 10,
			minPages: 0,
			minWidgets: 0,
			minTables: 0,
			minEvents: 2,
			requiredNodeCapabilities: [
				{
					alias: "imap_fetch",
					anyOf: ["email_imap_inbox_fetch_mail", "email_imap_connect"],
				},
				{ alias: "smtp_send", anyOf: ["email_smtp_send"] },
				{
					alias: "database_read",
					anyOf: [
						"filter_local_db",
						"list_local_db",
						"fts_search_local_db",
						"vector_search_local_db",
						"hybrid_search_local_db",
						"df_sql_query",
					],
				},
				{
					alias: "database_write",
					anyOf: [
						"insert_local_db",
						"upsert_local_db",
						"batch_insert_local_db",
						"batch_upsert_local_db",
					],
				},
			],
		}),
	},
	{
		id: "doc-compliance",
		title: "Document compliance pipeline with PII masking",
		description:
			"Exercises FlowPath file IO, document text extraction and regex/AI PII masking with an audit trail — no UI.",
		appName: "Doc Sentinel",
		smoke: false,
		prompt: `Build a document compliance pipeline that lives entirely in workflows — this app has NO pages and NO widgets.

One manual/scheduled entry processes a document from the app's upload storage: resolve the file path from the upload directory (leave the concrete file name as an empty string input for the user), read the file content as text, mask personally identifiable information (emails, phone numbers, names) with a PII masking node, and write the cleaned text back to storage under a "cleaned/" prefix. Persist an audit row for every processed document (document name, processed timestamp string, mask count or status) with a database write into a table named exactly "Compliance Audit". Handle a missing or unreadable file safely with a clear log message. Register the entry as an app event.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 700,
			minTotalNodes: 8,
			minPages: 0,
			minWidgets: 0,
			minTables: 0,
			minEvents: 1,
			requiredNodeCapabilities: [
				{
					alias: "pii_mask",
					anyOf: ["processing_pii_mask_regex", "processing_pii_mask_ai"],
				},
				{ alias: "file_read", anyOf: ["read_to_string", "read_to_bytes"] },
				{ alias: "file_write", anyOf: ["write_string", "write_bytes"] },
				{
					alias: "database_write",
					anyOf: [
						"insert_local_db",
						"upsert_local_db",
						"batch_insert_local_db",
						"batch_upsert_local_db",
					],
				},
			],
		}),
	},
	{
		id: "webhook-enrichment",
		title: "Webhook enrichment endpoint",
		description:
			"Exercises a Generic Event entry with payload parameters, outbound HTTP enrichment, JSON parsing and a structured event return — no UI.",
		appName: "Hook Enricher",
		smoke: false,
		prompt: `Build a webhook-style enrichment endpoint that lives entirely in workflows — this app has NO pages and NO widgets.

Create ONE Generic Event entry that accepts an inbound payload with parameters "lookupUrl" (string) and "subject" (string). The workflow must validate that both parameters are non-empty, fetch the lookup URL over HTTP, parse the response body as JSON, and build an enriched result struct combining the subject, the parsed response, and a processed-at marker string. Persist each enriched result with a database write into a table named exactly "Enrichment Log", and return the enriched result struct from the event with the generic event return node so the caller receives it. When validation or the fetch fails, return a struct with an "error" field describing the problem instead. Register the entry as an app event.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 700,
			minTotalNodes: 8,
			minPages: 0,
			minWidgets: 0,
			minTables: 0,
			minEvents: 1,
			requiredNodeCapabilities: [
				{ alias: "generic_event", anyOf: ["events_generic"] },
				{
					alias: "web_request",
					anyOf: ["http_fetch", "http_make_request", "streaming_http_fetch"],
				},
				{
					alias: "json_parse",
					anyOf: ["val_from_string", "parse_with_schema"],
				},
				{ alias: "event_return", anyOf: ["events_generic_return_result"] },
				{
					alias: "database_write",
					anyOf: [
						"insert_local_db",
						"upsert_local_db",
						"batch_insert_local_db",
						"batch_upsert_local_db",
					],
				},
			],
		}),
	},
	{
		id: "agent-tools",
		title: "Chat agent with registered function tools",
		description:
			"Exercises the agent framework: agent construction from a model, function-tool registration and agent invocation behind one chat entry — no UI.",
		appName: "Tool Agent",
		smoke: false,
		prompt: `Build a chat assistant powered by the agent framework — this app has NO pages and NO widgets; the chat surface is the whole UI, behind exactly ONE Chat Event entry (eventsChat).

Construct the agent from a model (find the model with the model-preference node and leave concrete preference inputs at their defaults for the user), set a concise system prompt describing the assistant, and register at least TWO function tools the agent can call: (1) a "saveNote" tool function that persists a note text with a database write into a table named exactly "Agent Notes", and (2) a "countNotes" tool function that reads the persisted notes back with a database read/filter node and returns how many exist. Invoke the agent with the incoming chat message and reply through the chat response with the agent's answer. Handle an empty chat message with a clear reply. Register the chat entry as an app event.`,
		requirements: requirements({
			minFlowScriptNonWhitespaceChars: 800,
			minTotalNodes: 10,
			minPages: 0,
			minWidgets: 0,
			minTables: 0,
			minEvents: 1,
			requiredNodeCapabilities: [
				{ alias: "chat_event", anyOf: ["events_chat"] },
				{ alias: "agent_core", anyOf: ["agent_from_model"] },
				{
					alias: "agent_tools",
					anyOf: [
						"agent_register_function_tools",
						"agent_lazy_register_function_tools",
					],
				},
				{
					alias: "agent_invoke",
					anyOf: ["agent_invoke", "agent_stream_invoke"],
				},
				{
					alias: "database_write",
					anyOf: [
						"insert_local_db",
						"upsert_local_db",
						"batch_insert_local_db",
						"batch_upsert_local_db",
					],
				},
			],
		}),
	},
] as const satisfies readonly FlowPilotE2ECaseDefinition[];

export const FLOWPILOT_APP_CREATION_SMOKE_CASES =
	FLOWPILOT_APP_CREATION_CASES.filter((caseDefinition) => caseDefinition.smoke);

export function getFlowPilotAppCreationCase(
	id: FlowPilotE2ECaseId,
): FlowPilotE2ECaseDefinition {
	const caseDefinition = FLOWPILOT_APP_CREATION_CASES.find(
		(candidate) => candidate.id === id,
	);
	if (!caseDefinition) {
		throw new Error(`Unknown FlowPilot app-creation E2E case: ${id}`);
	}
	return caseDefinition;
}

export function selectFlowPilotAppCreationCases(
	options: {
		smoke?: boolean;
		ids?: readonly FlowPilotE2ECaseId[];
	} = {},
): readonly FlowPilotE2ECaseDefinition[] {
	const selected = options.smoke
		? FLOWPILOT_APP_CREATION_SMOKE_CASES
		: FLOWPILOT_APP_CREATION_CASES;
	if (!options.ids) return selected;

	const requested = new Set(options.ids);
	return selected.filter((caseDefinition) => requested.has(caseDefinition.id));
}

export function resolveFlowPilotE2ERunCases(
	options: Pick<FlowPilotE2ERunOptions, "caseId" | "caseIds" | "suite"> = {},
): readonly FlowPilotE2ECaseDefinition[] {
	if (options.caseId && options.caseIds?.length) {
		throw new Error("Use either caseId or caseIds, not both.");
	}
	if ((options.caseId || options.caseIds?.length) && options.suite) {
		throw new Error("Use either explicit cases or a suite, not both.");
	}

	const ids = options.caseId ? [options.caseId] : options.caseIds;
	if (ids?.length) {
		const unique = [...new Set(ids)];
		return unique.map(getFlowPilotAppCreationCase);
	}

	return options.suite === "full"
		? FLOWPILOT_APP_CREATION_CASES
		: FLOWPILOT_APP_CREATION_SMOKE_CASES;
}

function normalizedRunSuffix(runSuffix: string): string {
	return runSuffix
		.replace(/\p{Cc}+/gu, " ")
		.trim()
		.replace(/\s+/g, " ");
}

export function buildCasePrompt(
	caseDefinition: FlowPilotE2ECaseDefinition,
	runSuffix = "",
	options: BuildFlowPilotE2EPromptOptions = {},
): BuiltFlowPilotE2EPrompt {
	const minChars =
		options.minFlowScriptNonWhitespaceChars ??
		caseDefinition.requirements.minFlowScriptNonWhitespaceChars;
	if (!Number.isSafeInteger(minChars) || minChars < 1) {
		throw new Error(
			"minFlowScriptNonWhitespaceChars must be a positive safe integer",
		);
	}
	if (minChars > caseDefinition.requirements.maxFlowScriptNonWhitespaceChars) {
		throw new Error(
			`minFlowScriptNonWhitespaceChars must not exceed the compactness ceiling (${caseDefinition.requirements.maxFlowScriptNonWhitespaceChars})`,
		);
	}

	const suffix = normalizedRunSuffix(runSuffix);
	const expectedAppName = suffix
		? `${caseDefinition.appName} ${suffix}`
		: caseDefinition.appName;
	const resolvedCase = {
		...caseDefinition,
		expectedAppName,
		requirements: {
			...caseDefinition.requirements,
			minFlowScriptNonWhitespaceChars: minChars,
		},
	};

	// Cases that require no pages/widgets/tables are pure code-generation exercises: steering the
	// orchestrator away from UI/Data-Studio specialists keeps the whole run budget on FlowScript.
	const needsScaffolding =
		resolvedCase.requirements.minPages > 0 ||
		resolvedCase.requirements.minWidgets > 0 ||
		resolvedCase.requirements.minTables > 0;
	const setupLine = needsScaffolding
		? "- Finish the UI and data setup, then implement every executable behavior as one compact, valid FlowScript program. Do not substitute command JSON or leave placeholder logic."
		: "- Skip page, widget, and Data Studio scaffolding entirely: go straight to the workflow board and implement every behavior as one compact, valid FlowScript program. Do not substitute command JSON or leave placeholder logic.";
	const idsLine = needsScaffolding
		? "- Use ids returned by the created tables, pages, widgets, and actions. Never guess an id. Preserve the requested entity names exactly so the run can resolve their semantic aliases."
		: "- Database tables are created automatically by the workflow's first write: use the exact requested table names directly in the insert/upsert calls, and use entity ids that come from real tool results.";
	const completionLine = needsScaffolding
		? "- Complete the whole app in this run and do not report success from UI/data scaffolding alone."
		: "- Complete the whole app in this run; the committed FlowScript program is the deliverable.";
	const verificationLine =
		"- Do NOT execute the workflow, start chat sessions, or run any other runtime verification in this benchmark: compile/lint/reconcile receipts are the only acceptance evidence. Once the FlowScript is committed and the required events are registered, stop and summarize.";
	const repairBudgetLine =
		"- REPAIR BUDGET: after the first board build returns a persisted result, delegate at most TWO follow-up board repairs. If problems remain after the second repair, stop and report them honestly instead of iterating further.";
	const contract = `FlowPilot E2E contract:
- Create the app named exactly ${JSON.stringify(expectedAppName)} as a LOCAL app: pass online: false to create_app. The benchmark must stay hermetic — cloud sync (e.g. remote event registration) is unavailable and would fail the run.
${setupLine}
- The authored FlowScript must compile, lint, reconcile, and persist. Read back and repair the canonical FlowScript until it has no error diagnostics.
- Keep working FlowScript as short as practical: no comments, padding, repeated helpers, dead branches, or prose. It must still contain at least ${minChars} non-whitespace characters and no more than ${caseDefinition.requirements.maxFlowScriptNonWhitespaceChars}; the lower threshold is only a truncation sanity check, so never pad to reach it.
${idsLine}
${verificationLine}
${repairBudgetLine}
${completionLine}`;

	return {
		caseDefinition: resolvedCase,
		expectedAppName,
		prompt: `${contract}\n\nScenario:\n${caseDefinition.prompt}`,
	};
}
