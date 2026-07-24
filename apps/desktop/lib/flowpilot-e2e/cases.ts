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
		prompt: `Build a research agent that lives entirely in chat: implement everything behind exactly ONE Chat Event entry (eventsChat) and answer through the chat response. Do not add eventsSimple or eventsGeneric entries — branch on the incoming message inside the workflow, using functions for the sub-behaviors. Do not build any pages or widgets — the chat surface is the whole UI — and do not pre-create database tables: the tables are defined by the workflow's first write.

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

Use tables named exactly "Expense Requests" and "Expense Audit". The workflow must load pending expenses, approve or reject exactly one request, require a rejection reason, prevent a request from being decided twice, append an audit record, and refresh the queue. Wire both real widget action ids and persisted table ids into the workflow.`,
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

Use tables named exactly "Incidents" and "Incident Updates". Support creating an incident from an incoming/manual event, loading active incidents, acknowledging once, resolving with a summary, and appending every transition to the timeline. Reject invalid state transitions and make repeated action delivery idempotent. Wire the real page, table, widget, and widget-action ids.`,
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
		: "- Database tables are defined by the workflow's first write: use the exact requested table names in the insert/upsert calls instead of pre-creating tables, and never guess entity ids.";
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
