import {
	isSuccessfulFlowScriptCheckReceipt,
	isSuccessfulFlowScriptCommitReceipt,
} from "@flow-like/flow-like-ui/lib/flowpilot/flowscript-generation-receipt";

import { FLOWPILOT_E2E_DEFAULT_MODEL } from "./cases";
import { findExactSuccessfulCompilerPair } from "./receipt-evidence";
import type {
	FlowPilotAppCreationSnapshot,
	FlowPilotBoardSnapshot,
	FlowPilotE2ECaseDefinition,
	FlowPilotE2ECheck,
	FlowPilotE2EEntityKind,
	FlowPilotE2ERunReport,
	FlowPilotEventSnapshot,
	FlowPilotPageSnapshot,
	FlowPilotTableSnapshot,
	FlowPilotWidgetActionSnapshot,
	FlowPilotWidgetSnapshot,
	FlowScriptReferenceSource,
	FlowScriptSizeMetrics,
	ResolvedFlowPilotE2ECase,
} from "./types";

interface ResolvableEntity {
	id: string;
	aliases: readonly string[];
	path: string;
}

const WIDGET_ID_KEYS = new Set(["widgetId", "widget_id"]);
const PAGE_ID_KEYS = new Set([
	"pageId",
	"page_id",
	"targetPageId",
	"target_page_id",
	"defaultPageId",
	"default_page_id",
]);
const NON_SUCCESS_AUTHORED_STATUSES = new Set([
	"validation_errors",
	"interrupted",
	"drafting",
	"submitted",
	"stale",
	"failed",
	"error",
	"cancelled",
	"canceled",
	"timed_out",
	"timeout",
	"partial",
	"partial_working_slice",
	"regression_blocked",
]);
const NON_SUCCESS_AUTHORED_COMPLETIONS = new Set([
	"partial",
	"partial_working_slice",
	"regression_blocked",
]);

function normalizedLifecycleValue(value: string): string {
	return value
		.trim()
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, "_")
		.replace(/^_+|_+$/g, "");
}

function byId<T extends { id: string }>(left: T, right: T): number {
	return left.id.localeCompare(right.id);
}

function check(
	code: string,
	passed: boolean,
	message: string,
	options: Pick<FlowPilotE2ECheck, "path" | "expected" | "actual"> = {},
): FlowPilotE2ECheck {
	return {
		code,
		status: passed ? "pass" : "fail",
		message,
		...options,
	};
}

export function normalizeSemanticAlias(value: string): string {
	return value
		.normalize("NFKD")
		.replace(/\p{M}+/gu, "")
		.toLowerCase()
		.replace(/&/g, " and ")
		.replace(/[^a-z0-9]+/g, "_")
		.replace(/^_+|_+$/g, "");
}

export function flowScriptSizeMetrics(source: string): FlowScriptSizeMetrics {
	const characters = source.length;
	return {
		characters,
		nonWhitespaceCharacters: source.replace(/\s/g, "").length,
		lines: characters === 0 ? 0 : source.split(/\r\n|\r|\n/).length,
		estimatedTokens: characters === 0 ? 0 : Math.ceil(characters / 4),
	};
}

function definedSource(source: string | undefined): string | undefined {
	return source && source.trim().length > 0 ? source : undefined;
}

function authoredSource(
	snapshot: FlowPilotAppCreationSnapshot,
	boards: readonly FlowPilotBoardSnapshot[],
): string | undefined {
	const appSource = definedSource(snapshot.authoredFlowScript);
	if (appSource) return appSource;
	const boardSources = boards
		.map((board) => definedSource(board.authoredFlowScript))
		.filter((source): source is string => source !== undefined);
	return boardSources.length > 0 ? boardSources.join("\n") : undefined;
}

function tableEntity(
	table: string | FlowPilotTableSnapshot,
	index: number,
): ResolvableEntity {
	if (typeof table === "string") {
		return { id: table, aliases: [table], path: `tables[${index}]` };
	}
	return {
		id: table.id,
		aliases: [table.semanticAlias ?? "", table.name, table.id],
		path: `tables[${index}]`,
	};
}

function namedEntity(
	entity: FlowPilotPageSnapshot | FlowPilotWidgetSnapshot,
	path: string,
): ResolvableEntity {
	return {
		id: entity.id,
		aliases: [entity.semanticAlias ?? "", entity.name, entity.id],
		path,
	};
}

function eventEntity(
	event: FlowPilotEventSnapshot,
	index: number,
): ResolvableEntity | undefined {
	const id = event.id ?? event.nodeId;
	if (!id) return undefined;
	return {
		id,
		aliases: [
			event.semanticAlias ?? "",
			event.name ?? "",
			event.id ?? "",
			event.nodeId ?? "",
		],
		path: `events[${index}]`,
	};
}

function actionEntity(
	action: string | FlowPilotWidgetActionSnapshot,
	widget: FlowPilotWidgetSnapshot,
	widgetIndex: number,
	actionIndex: number,
): ResolvableEntity {
	const path = `widgets[${widgetIndex}].actions[${actionIndex}]`;
	if (typeof action === "string") {
		return {
			id: action,
			aliases: [action, `${widget.name}_${action}`],
			path,
		};
	}
	return {
		id: action.id,
		aliases: [
			action.semanticAlias ?? "",
			action.name ?? "",
			action.label ?? "",
			action.id,
			`${widget.name}_${action.name ?? action.label ?? action.id}`,
		],
		path,
	};
}

function entitiesByKind(
	kind: FlowPilotE2EEntityKind,
	snapshot: FlowPilotAppCreationSnapshot,
): readonly ResolvableEntity[] {
	switch (kind) {
		case "page":
			return snapshot.pages.map((page, index) =>
				namedEntity(page, `pages[${index}]`),
			);
		case "widget":
			return snapshot.widgets.map((widget, index) =>
				namedEntity(widget, `widgets[${index}]`),
			);
		case "widget_action":
			return snapshot.widgets.flatMap((widget, widgetIndex) =>
				(widget.actions ?? []).map((action, actionIndex) =>
					actionEntity(action, widget, widgetIndex, actionIndex),
				),
			);
		case "table":
			return snapshot.tables.map(tableEntity);
		case "event":
			return snapshot.events
				.map(eventEntity)
				.filter((event): event is ResolvableEntity => event !== undefined);
	}
}

function resolveEntity(
	kind: FlowPilotE2EEntityKind,
	alias: string,
	snapshot: FlowPilotAppCreationSnapshot,
): { entity?: ResolvableEntity; ambiguous: boolean } {
	const normalizedAlias = normalizeSemanticAlias(alias);
	const matches = entitiesByKind(kind, snapshot).filter((entity) =>
		entity.aliases.some(
			(candidate) => normalizeSemanticAlias(candidate) === normalizedAlias,
		),
	);
	const uniqueMatches = [
		...new Map(matches.map((entity) => [entity.id, entity])).values(),
	];
	return {
		entity: uniqueMatches.length === 1 ? uniqueMatches[0] : undefined,
		ambiguous: uniqueMatches.length > 1,
	};
}

function flowScriptStringValues(source: string | undefined): readonly string[] {
	if (!source) return [];
	const values: string[] = [];
	for (const match of source.matchAll(/"(?:\\.|[^"\\])*"/g)) {
		try {
			const value = JSON.parse(match[0]);
			if (typeof value === "string") values.push(value);
		} catch {
			// Native lint reports malformed literals; reference matching stays conservative.
		}
	}
	return values;
}

function sourceContainsReference(
	kind: FlowPilotE2EEntityKind,
	source: FlowScriptReferenceSource,
	id: string,
	authored: string | undefined,
	canonical: string,
): boolean {
	// Persisted ids must be wired as string values, not merely appear inside a helper/function
	// name. A2UI page element targets are encoded as "<page-id>/<element-id>" and therefore count
	// as a concrete page reference; all other entity kinds require an exact string value.
	const matches = (value: string) =>
		value === id || (kind === "page" && value.startsWith(`${id}/`));
	const inAuthored = flowScriptStringValues(authored).some(matches);
	const inCanonical = flowScriptStringValues(canonical).some(matches);
	switch (source) {
		case "authored":
			return inAuthored;
		case "canonical":
			return inCanonical;
		case "both":
			return inAuthored && inCanonical;
		case "either":
			return inAuthored || inCanonical;
	}
}

function duplicateIds(items: readonly { id: string }[]): readonly string[] {
	const seen = new Set<string>();
	const duplicates = new Set<string>();
	for (const item of items) {
		if (seen.has(item.id)) duplicates.add(item.id);
		seen.add(item.id);
	}
	return [...duplicates].sort();
}

function collectKeyedReferences(
	value: unknown,
	keys: ReadonlySet<string>,
): readonly string[] {
	const references = new Set<string>();
	const visited = new WeakSet<object>();

	function visit(current: unknown): void {
		if (!current || typeof current !== "object") return;
		if (visited.has(current)) return;
		visited.add(current);

		if (Array.isArray(current)) {
			for (const item of current) visit(item);
			return;
		}

		for (const [key, nested] of Object.entries(current)) {
			if (keys.has(key) && typeof nested === "string" && nested.length > 0) {
				references.add(nested);
			}
			visit(nested);
		}
	}

	visit(value);
	return [...references].sort();
}

function collectWidgetRefs(page: FlowPilotPageSnapshot): readonly string[] {
	const references = new Set(
		collectKeyedReferences(page.content, WIDGET_ID_KEYS),
	);
	const widgetRefs = page.widgetRefs;
	if (!widgetRefs) return [...references].sort();

	const values = Array.isArray(widgetRefs)
		? widgetRefs
		: Object.values(widgetRefs);
	for (const value of values) {
		if (typeof value === "string") {
			references.add(value);
			continue;
		}
		if (!value || typeof value !== "object") continue;
		const record = value as Record<string, unknown>;
		const id = record.widgetId ?? record.widget_id ?? record.id;
		if (typeof id === "string" && id.length > 0) references.add(id);
	}
	return [...references].sort();
}

function integrityChecks(
	snapshot: FlowPilotAppCreationSnapshot,
	boards: readonly FlowPilotBoardSnapshot[],
	pages: readonly FlowPilotPageSnapshot[],
	widgets: readonly FlowPilotWidgetSnapshot[],
): readonly FlowPilotE2ECheck[] {
	const checks: FlowPilotE2ECheck[] = [];
	const boardIds = new Set(boards.map((board) => board.id));
	const pageIds = new Set(pages.map((page) => page.id));
	const widgetIds = new Set(widgets.map((widget) => widget.id));
	const allNodeIds = new Set(boards.flatMap((board) => board.nodeIds ?? []));
	const nodeIdsForBoard = (
		boardId: string | undefined,
	): ReadonlySet<string> => {
		if (!boardId) return allNodeIds;
		return new Set(boards.find((board) => board.id === boardId)?.nodeIds ?? []);
	};

	for (const [label, items] of [
		["board", boards],
		["page", pages],
		["widget", widgets],
	] as const) {
		const duplicates = duplicateIds(items);
		checks.push(
			check(
				`integrity.${label}_ids_unique`,
				duplicates.length === 0,
				duplicates.length === 0
					? `All ${label} ids are unique.`
					: `Duplicate ${label} ids: ${duplicates.join(", ")}.`,
				{ actual: duplicates.join(", ") },
			),
		);
	}

	for (const page of pages) {
		if (page.boardId) {
			checks.push(
				check(
					`integrity.page_board.${page.id}`,
					boardIds.has(page.boardId),
					boardIds.has(page.boardId)
						? `Page ${page.id} references an existing board.`
						: `Page ${page.id} references missing board ${page.boardId}.`,
					{ path: `pages.${page.id}.boardId`, actual: page.boardId },
				),
			);
		}

		for (const [binding, nodeId] of [
			["load", page.onLoadEventId],
			["unload", page.onUnloadEventId],
			["interval", page.onIntervalEventId],
		] as const) {
			if (!nodeId) continue;
			const boardNodeIds = nodeIdsForBoard(page.boardId);
			checks.push(
				check(
					`integrity.page_${binding}_node.${page.id}`,
					boardNodeIds.has(nodeId),
					boardNodeIds.has(nodeId)
						? `Page ${page.id} references an existing ${binding} node.`
						: `Page ${page.id} references missing ${binding} node ${nodeId}.`,
					{
						path: `pages.${page.id}.on${binding[0].toUpperCase()}${binding.slice(1)}EventId`,
						actual: nodeId,
					},
				),
			);
		}

		for (const widgetId of collectWidgetRefs(page)) {
			checks.push(
				check(
					`integrity.page_widget.${page.id}.${widgetId}`,
					widgetIds.has(widgetId),
					widgetIds.has(widgetId)
						? `Page ${page.id} references existing widget ${widgetId}.`
						: `Page ${page.id} references missing widget ${widgetId}.`,
					{ path: `pages.${page.id}.content`, actual: widgetId },
				),
			);
		}

		for (const pageId of collectKeyedReferences(page.content, PAGE_ID_KEYS)) {
			checks.push(
				check(
					`integrity.page_reference.${page.id}.${pageId}`,
					pageIds.has(pageId),
					pageIds.has(pageId)
						? `Page ${page.id} references existing page ${pageId}.`
						: `Page ${page.id} references missing page ${pageId}.`,
					{ path: `pages.${page.id}.content`, actual: pageId },
				),
			);
		}
	}

	for (const [index, event] of snapshot.events.entries()) {
		if (event.boardId) {
			checks.push(
				check(
					`integrity.event_board.${event.id ?? event.nodeId ?? index}`,
					boardIds.has(event.boardId),
					boardIds.has(event.boardId)
						? `Event ${event.id ?? index} references an existing board.`
						: `Event ${event.id ?? index} references missing board ${event.boardId}.`,
					{ path: `events[${index}].boardId`, actual: event.boardId },
				),
			);
		}
		if (event.pageId) {
			checks.push(
				check(
					`integrity.event_page.${event.id ?? event.nodeId ?? index}`,
					pageIds.has(event.pageId),
					pageIds.has(event.pageId)
						? `Event ${event.id ?? index} references an existing page.`
						: `Event ${event.id ?? index} references missing page ${event.pageId}.`,
					{ path: `events[${index}].pageId`, actual: event.pageId },
				),
			);
		}
		if (event.nodeId) {
			const boardNodeIds = nodeIdsForBoard(event.boardId);
			checks.push(
				check(
					`integrity.event_node.${event.id ?? index}`,
					boardNodeIds.has(event.nodeId),
					boardNodeIds.has(event.nodeId)
						? `Event ${event.id ?? index} references an existing board node.`
						: `Event ${event.id ?? index} references missing board node ${event.nodeId}.`,
					{ path: `events[${index}].nodeId`, actual: event.nodeId },
				),
			);
		}
	}

	return checks;
}

function expectedAppName(
	caseDefinition: FlowPilotE2ECaseDefinition | ResolvedFlowPilotE2ECase,
): string {
	return "expectedAppName" in caseDefinition
		? caseDefinition.expectedAppName
		: caseDefinition.appName;
}

export function evaluateAppCreationCase(
	caseDefinition: FlowPilotE2ECaseDefinition | ResolvedFlowPilotE2ECase,
	snapshot: FlowPilotAppCreationSnapshot,
): FlowPilotE2ERunReport {
	const requirements = caseDefinition.requirements;
	const expectedName = expectedAppName(caseDefinition);
	const boards = [...snapshot.boards].sort(byId);
	const pages = [...snapshot.pages].sort(byId);
	const widgets = [...snapshot.widgets].sort(byId);
	const authored = authoredSource(snapshot, boards);
	const canonicalBoards = boards.filter((board) =>
		definedSource(board.flowScript),
	);
	const canonicalSource = canonicalBoards
		.map((board) => board.flowScript as string)
		.join("\n");
	const canonicalMetrics = canonicalBoards.map((board) => ({
		boardId: board.id,
		boardName: board.name,
		...flowScriptSizeMetrics(board.flowScript as string),
	}));
	const totalNodes = boards.reduce(
		(total, board) =>
			total + Math.max(0, board.nodeCount ?? board.nodeIds?.length ?? 0),
		0,
	);
	const checks: FlowPilotE2ECheck[] = [];

	checks.push(
		check(
			"app.id",
			snapshot.appId.trim().length > 0,
			"The app has a persisted id.",
			{
				path: "appId",
				actual: snapshot.appId,
			},
		),
		check(
			"app.name",
			snapshot.appName === expectedName,
			snapshot.appName === expectedName
				? `App name exactly matches ${JSON.stringify(expectedName)}.`
				: `Expected app name ${JSON.stringify(expectedName)}, got ${JSON.stringify(snapshot.appName)}.`,
			{ path: "appName", expected: expectedName, actual: snapshot.appName },
		),
	);
	checks.push(
		check(
			"model.present",
			Boolean(snapshot.model),
			snapshot.model
				? "Generation model configuration was captured."
				: "Generation model configuration is missing.",
			{ expected: true, actual: Boolean(snapshot.model) },
		),
		check(
			"model.provider",
			snapshot.model?.provider === FLOWPILOT_E2E_DEFAULT_MODEL.provider,
			`Generation provider must be ${FLOWPILOT_E2E_DEFAULT_MODEL.provider}.`,
			{
				expected: FLOWPILOT_E2E_DEFAULT_MODEL.provider,
				actual: snapshot.model?.provider ?? "missing",
			},
		),
		check(
			"model.id",
			snapshot.model?.model === FLOWPILOT_E2E_DEFAULT_MODEL.model,
			`Generation model must be ${FLOWPILOT_E2E_DEFAULT_MODEL.model}.`,
			{
				expected: FLOWPILOT_E2E_DEFAULT_MODEL.model,
				actual: snapshot.model?.model ?? "missing",
			},
		),
		check(
			"model.reasoning_effort",
			snapshot.model?.reasoningEffort ===
				FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort,
			`Generation reasoning effort must be ${FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort}.`,
			{
				expected: FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort,
				actual: snapshot.model?.reasoningEffort ?? "missing",
			},
		),
	);

	const generationRuns = snapshot.flowScriptGenerationRuns ?? [];
	const compilerPairs = generationRuns.flatMap((run) => {
		const pair = findExactSuccessfulCompilerPair(run);
		return pair ? [{ run, ...pair }] : [];
	});
	if (requirements.requireSuccessfulCompilerReceipt) {
		const boardIds = new Set(boards.map((board) => board.id));
		const scopedRuns = generationRuns.filter(
			(run) => run.appId === snapshot.appId && boardIds.has(run.boardId),
		);
		const expectedNestedModelId = `${FLOWPILOT_E2E_DEFAULT_MODEL.provider}:${FLOWPILOT_E2E_DEFAULT_MODEL.model}`;
		const modelMatchedRuns = scopedRuns.filter(
			(run) =>
				run.provider === FLOWPILOT_E2E_DEFAULT_MODEL.provider &&
				run.modelId === expectedNestedModelId &&
				run.reasoningEffort === FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort,
		);
		const successfulChecks = generationRuns.flatMap((run) =>
			run.compilerReceipts.filter(isSuccessfulFlowScriptCheckReceipt),
		);
		const successfulCommits = generationRuns.flatMap((run) =>
			run.compilerReceipts.filter(isSuccessfulFlowScriptCommitReceipt),
		);
		const nonemptyBoards = boards.filter(
			(board) => Math.max(0, board.nodeCount ?? board.nodeIds?.length ?? 0) > 0,
		);
		const pairedBoardIds = new Set(compilerPairs.map(({ run }) => run.boardId));
		const allPairsHaveRawCandidate = compilerPairs.every(({ run, commit }) =>
			run.candidates.some((candidate) => candidate.source === commit.source),
		);
		const allPairsHaveFullReceipt = compilerPairs.every(
			({ check: checked, commit }) =>
				Boolean(checked.baseFingerprint) &&
				checked.derivedCommandCount !== undefined &&
				commit.derivedCommandCount !== undefined &&
				commit.queuedCount !== undefined,
		);
		checks.push(
			check(
				"flowscript.compiler_receipt.present",
				generationRuns.length > 0,
				generationRuns.length > 0
					? `Captured ${generationRuns.length} board generation run receipt(s).`
					: "No board generation receipt was captured.",
				{ expected: true, actual: generationRuns.length > 0 },
			),
			check(
				"flowscript.compiler_receipt.board_scope",
				scopedRuns.length === generationRuns.length,
				scopedRuns.length === generationRuns.length
					? "Every generation receipt belongs to the created app and a persisted board."
					: `${generationRuns.length - scopedRuns.length} generation receipt(s) reference another app or a missing board.`,
				{
					expected: generationRuns.length,
					actual: scopedRuns.length,
				},
			),
			check(
				"flowscript.compiler_receipt.nested_model",
				scopedRuns.length > 0 && modelMatchedRuns.length === scopedRuns.length,
				scopedRuns.length > 0 && modelMatchedRuns.length === scopedRuns.length
					? `Every scoped compiler run used ${FLOWPILOT_E2E_DEFAULT_MODEL.provider}/${expectedNestedModelId}/${FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort}.`
					: `${scopedRuns.length - modelMatchedRuns.length} scoped compiler run(s) did not use ${FLOWPILOT_E2E_DEFAULT_MODEL.provider}/${expectedNestedModelId}/${FLOWPILOT_E2E_DEFAULT_MODEL.reasoningEffort}.`,
				{
					expected: scopedRuns.length,
					actual: modelMatchedRuns.length,
				},
			),
			check(
				"flowscript.compiler_receipt.check_success",
				successfulChecks.length > 0,
				successfulChecks.length > 0
					? `Captured ${successfulChecks.length} clean check_flowscript receipt(s).`
					: "No clean check_flowscript receipt with its exact authored source was captured.",
				{ expected: true, actual: successfulChecks.length > 0 },
			),
			check(
				"flowscript.compiler_receipt.commit_success",
				successfulCommits.length > 0,
				successfulCommits.length > 0
					? `Captured ${successfulCommits.length} clean commit receipt(s).`
					: "No clean commit_flowscript/edit_flowscript receipt with its exact authored source was captured.",
				{ expected: true, actual: successfulCommits.length > 0 },
			),
			check(
				"flowscript.compiler_receipt.exact_revision",
				compilerPairs.length > 0,
				compilerPairs.length > 0
					? `${compilerPairs.length} committed source revision(s) match an earlier successful check exactly.`
					: "No committed source matches a successful check_flowscript receipt by source, draft id, and revision.",
				{ expected: true, actual: compilerPairs.length > 0 },
			),
			check(
				"flowscript.compiler_receipt.applied_readback",
				compilerPairs.length > 0,
				compilerPairs.length > 0
					? `${compilerPairs.length} exact compiler pair(s) finished successfully with persisted readback.`
					: "No exact compiler pair finished with outcome ok and verified persisted readback.",
				{ expected: true, actual: compilerPairs.length > 0 },
			),
			check(
				"flowscript.compiler_receipt.raw_candidate",
				compilerPairs.length > 0 && allPairsHaveRawCandidate,
				compilerPairs.length > 0 && allPairsHaveRawCandidate
					? "Every successful compiler pair retains its byte-for-byte model-authored candidate."
					: "A successful compiler pair is missing its byte-for-byte authored candidate.",
				{ expected: true, actual: allPairsHaveRawCandidate },
			),
			check(
				"flowscript.compiler_receipt.full_envelope",
				compilerPairs.length > 0 && allPairsHaveFullReceipt,
				compilerPairs.length > 0 && allPairsHaveFullReceipt
					? "Successful compiler receipts include fingerprint and command-count evidence."
					: "A successful compiler receipt is missing its fingerprint or command-count evidence.",
				{ expected: true, actual: allPairsHaveFullReceipt },
			),
		);
		for (const board of nonemptyBoards) {
			checks.push(
				check(
					`flowscript.compiler_receipt.board.${board.id}`,
					pairedBoardIds.has(board.id),
					pairedBoardIds.has(board.id)
						? `Board ${board.id} has its own successful exact check→commit pair and persisted readback.`
						: `Nonempty board ${board.id} has no successful exact check→commit pair with persisted readback.`,
					{
						path: `boards.${board.id}`,
						expected: true,
						actual: pairedBoardIds.has(board.id),
					},
				),
			);
		}
	}

	for (const [code, actual, minimum] of [
		["boards.count", boards.length, requirements.minBoards],
		["boards.total_nodes", totalNodes, requirements.minTotalNodes],
		["pages.count", pages.length, requirements.minPages],
		["widgets.count", widgets.length, requirements.minWidgets],
		["tables.count", snapshot.tables.length, requirements.minTables],
		["events.count", snapshot.events.length, requirements.minEvents],
	] as const) {
		checks.push(
			check(
				code,
				actual >= minimum,
				actual >= minimum
					? `${code} meets its minimum (${actual} >= ${minimum}).`
					: `${code} is below its minimum (${actual} < ${minimum}).`,
				{ expected: minimum, actual },
			),
		);
	}
	for (const board of boards) {
		const nodeCount = Math.max(
			0,
			board.nodeCount ?? board.nodeIds?.length ?? 0,
		);
		checks.push(
			check(
				`boards.nonempty.${board.id}`,
				nodeCount > 0,
				nodeCount > 0
					? `Board ${board.id} contains ${nodeCount} node(s).`
					: `Board ${board.id} is empty.`,
				{ path: `boards.${board.id}`, expected: true, actual: nodeCount > 0 },
			),
		);
	}

	if (requirements.requireAuthoredFlowScript) {
		checks.push(
			check(
				"flowscript.authored.present",
				Boolean(authored),
				authored
					? "Captured the model-authored FlowScript."
					: "No model-authored FlowScript was captured.",
				{
					path: "authoredFlowScript",
					expected: true,
					actual: Boolean(authored),
				},
			),
		);
	}
	if (authored) {
		const metrics = flowScriptSizeMetrics(authored);
		const compactnessNoise = authored.split(/\r\n|\r|\n/).filter((line) => {
			const trimmed = line.trim();
			return (
				trimmed.startsWith("```") ||
				trimmed.startsWith("/*") ||
				(trimmed.startsWith("//") && !/^\/\/\s*@/.test(trimmed))
			);
		});
		checks.push(
			check(
				"flowscript.authored.min_non_whitespace",
				metrics.nonWhitespaceCharacters >=
					requirements.minFlowScriptNonWhitespaceChars,
				`Authored FlowScript has ${metrics.nonWhitespaceCharacters} non-whitespace characters (minimum ${requirements.minFlowScriptNonWhitespaceChars}).`,
				{
					expected: requirements.minFlowScriptNonWhitespaceChars,
					actual: metrics.nonWhitespaceCharacters,
				},
			),
			check(
				"flowscript.authored.max_non_whitespace",
				metrics.nonWhitespaceCharacters <=
					requirements.maxFlowScriptNonWhitespaceChars,
				`Authored FlowScript has ${metrics.nonWhitespaceCharacters} non-whitespace characters (maximum ${requirements.maxFlowScriptNonWhitespaceChars}).`,
				{
					expected: requirements.maxFlowScriptNonWhitespaceChars,
					actual: metrics.nonWhitespaceCharacters,
				},
			),
			check(
				"flowscript.authored.compact_source",
				compactnessNoise.length === 0,
				compactnessNoise.length === 0
					? "Authored FlowScript contains no Markdown fences or prose-comment padding."
					: `Authored FlowScript contains ${compactnessNoise.length} Markdown fence or prose-comment line(s).`,
				{ actual: compactnessNoise.length, expected: 0 },
			),
		);
	}
	if (snapshot.authoredFlowScriptStatus) {
		const normalizedStatus = normalizedLifecycleValue(
			snapshot.authoredFlowScriptStatus,
		);
		checks.push(
			check(
				"flowscript.authored.status",
				!NON_SUCCESS_AUTHORED_STATUSES.has(normalizedStatus),
				NON_SUCCESS_AUTHORED_STATUSES.has(normalizedStatus)
					? `Authored FlowScript ended in non-success status ${snapshot.authoredFlowScriptStatus}.`
					: `Authored FlowScript status is ${snapshot.authoredFlowScriptStatus}.`,
				{
					path: "authoredFlowScriptStatus",
					actual: snapshot.authoredFlowScriptStatus,
				},
			),
		);
	}
	if (snapshot.authoredFlowScriptCompletion) {
		const normalizedCompletion = normalizedLifecycleValue(
			snapshot.authoredFlowScriptCompletion,
		);
		checks.push(
			check(
				"flowscript.authored.completion",
				!NON_SUCCESS_AUTHORED_COMPLETIONS.has(normalizedCompletion),
				NON_SUCCESS_AUTHORED_COMPLETIONS.has(normalizedCompletion)
					? `Authored FlowScript ended as incomplete: ${snapshot.authoredFlowScriptCompletion}.`
					: `Authored FlowScript completion is ${snapshot.authoredFlowScriptCompletion}.`,
				{
					path: "authoredFlowScriptCompletion",
					actual: snapshot.authoredFlowScriptCompletion,
				},
			),
		);
	}
	if (requirements.requireAuthoredLintDiagnostics) {
		checks.push(
			check(
				"flowscript.authored.lint_available",
				Array.isArray(snapshot.authoredLintDiagnostics),
				Array.isArray(snapshot.authoredLintDiagnostics)
					? "Authored-source lint diagnostics were captured."
					: "Authored-source lint diagnostics are missing.",
				{ path: "authoredLintDiagnostics" },
			),
		);
	}
	if (snapshot.authoredLintDiagnostics) {
		const authoredErrors = snapshot.authoredLintDiagnostics.filter(
			(diagnostic) => diagnostic.severity.toLowerCase() === "error",
		);
		checks.push(
			check(
				"flowscript.authored.lint_errors",
				authoredErrors.length === 0,
				authoredErrors.length === 0
					? "Authored FlowScript has no lint errors."
					: `Authored FlowScript has ${authoredErrors.length} lint error(s): ${authoredErrors.map((error) => error.message).join(" | ")}`,
				{ actual: authoredErrors.length, expected: 0 },
			),
		);
	}

	if (requirements.requireCanonicalFlowScript) {
		checks.push(
			check(
				"flowscript.canonical.present",
				canonicalBoards.length > 0,
				canonicalBoards.length > 0
					? `Read canonical FlowScript from ${canonicalBoards.length} board(s).`
					: "No canonical FlowScript was read from a persisted board.",
				{ expected: true, actual: canonicalBoards.length > 0 },
			),
		);
		for (const board of boards) {
			checks.push(
				check(
					`flowscript.canonical.board_present.${board.id}`,
					Boolean(definedSource(board.flowScript)),
					definedSource(board.flowScript)
						? `Canonical FlowScript was read from board ${board.id}.`
						: `Canonical FlowScript is missing from persisted board ${board.id}.`,
					{ path: `boards.${board.id}.flowScript` },
				),
			);
		}
	}
	if (canonicalBoards.length > 0) {
		const largestCanonical = Math.max(
			...canonicalMetrics.map((metrics) => metrics.nonWhitespaceCharacters),
		);
		checks.push(
			check(
				"flowscript.canonical.min_non_whitespace",
				largestCanonical >= requirements.minFlowScriptNonWhitespaceChars,
				`Largest canonical FlowScript has ${largestCanonical} non-whitespace characters (minimum ${requirements.minFlowScriptNonWhitespaceChars}).`,
				{
					expected: requirements.minFlowScriptNonWhitespaceChars,
					actual: largestCanonical,
				},
			),
		);
	}

	for (const board of boards) {
		const diagnostics = board.lintDiagnostics;
		if (requirements.requireLintDiagnostics) {
			checks.push(
				check(
					`flowscript.lint.available.${board.id}`,
					Array.isArray(diagnostics),
					Array.isArray(diagnostics)
						? `Lint diagnostics were captured for board ${board.id}.`
						: `Lint diagnostics are missing for board ${board.id}.`,
					{ path: `boards.${board.id}.lintDiagnostics` },
				),
			);
		}
		if (diagnostics) {
			const errors = diagnostics.filter(
				(diagnostic) => diagnostic.severity.toLowerCase() === "error",
			);
			checks.push(
				check(
					`flowscript.lint.errors.${board.id}`,
					errors.length === 0,
					errors.length === 0
						? `Board ${board.id} has no FlowScript lint errors.`
						: `Board ${board.id} has ${errors.length} FlowScript lint error(s): ${errors.map((error) => error.message).join(" | ")}`,
					{ path: `boards.${board.id}.lintDiagnostics`, actual: errors.length },
				),
			);
		}

		const reconcile = board.reconcile;
		if (requirements.requireAuthoritativeReconcile) {
			checks.push(
				check(
					`flowscript.reconcile.available.${board.id}`,
					Boolean(reconcile),
					reconcile
						? `Authoritative reconcile result was captured for board ${board.id}.`
						: `Authoritative reconcile result is missing for board ${board.id}.`,
					{ path: `boards.${board.id}.reconcile` },
				),
			);
		}
		if (reconcile) {
			checks.push(
				check(
					`flowscript.reconcile.parse_valid.${board.id}`,
					reconcile.parseValid,
					`Board ${board.id} authoritative parseValid=${reconcile.parseValid}.`,
					{ actual: reconcile.parseValid, expected: true },
				),
				check(
					`flowscript.reconcile.reconcile_valid.${board.id}`,
					reconcile.reconcileValid,
					`Board ${board.id} authoritative reconcileValid=${reconcile.reconcileValid}.`,
					{ actual: reconcile.reconcileValid, expected: true },
				),
			);
			if (reconcile.idempotent !== undefined) {
				checks.push(
					check(
						`flowscript.reconcile.idempotent.${board.id}`,
						reconcile.idempotent,
						`Board ${board.id} authoritative idempotent=${reconcile.idempotent}.`,
						{ actual: reconcile.idempotent, expected: true },
					),
				);
			}
			if (reconcile.commandCount !== undefined) {
				checks.push(
					check(
						`flowscript.reconcile.commands.${board.id}`,
						reconcile.commandCount === 0,
						`Canonical board ${board.id} reconciled to ${reconcile.commandCount} command(s); idempotent readback requires 0.`,
						{ actual: reconcile.commandCount, expected: 0 },
					),
				);
			}
			if (reconcile.diagnostics) {
				checks.push(
					check(
						`flowscript.reconcile.diagnostics.${board.id}`,
						reconcile.diagnostics.length === 0,
						reconcile.diagnostics.length === 0
							? `Board ${board.id} reconcile result has no diagnostics.`
							: `Board ${board.id} reconcile result has ${reconcile.diagnostics.length} diagnostic(s): ${reconcile.diagnostics.join(" | ")}`,
						{ actual: reconcile.diagnostics.length, expected: 0 },
					),
				);
			}
		}
	}

	for (const alias of requirements.requiredSemanticTableAliases) {
		const resolution = resolveEntity("table", alias, snapshot);
		checks.push(
			check(
				`tables.semantic_alias.${normalizeSemanticAlias(alias)}`,
				Boolean(resolution.entity) && !resolution.ambiguous,
				resolution.ambiguous
					? `Semantic table alias ${JSON.stringify(alias)} is ambiguous.`
					: resolution.entity
						? `Resolved semantic table alias ${JSON.stringify(alias)} to ${resolution.entity.id}.`
						: `Missing semantic table alias ${JSON.stringify(alias)}.`,
				{ expected: alias, actual: resolution.entity?.id ?? "unresolved" },
			),
		);
	}

	const persistedNodeTypes = new Set(
		boards
			.flatMap((board) => board.nodeTypes ?? [])
			.map((nodeType) => nodeType.trim().toLowerCase()),
	);
	for (const capability of requirements.requiredNodeCapabilities) {
		const alternatives = capability.anyOf.map((nodeType) =>
			nodeType.trim().toLowerCase(),
		);
		const matched = alternatives.find((nodeType) =>
			persistedNodeTypes.has(nodeType),
		);
		checks.push(
			check(
				`flowscript.capability.${normalizeSemanticAlias(capability.alias)}`,
				Boolean(matched),
				matched
					? `Persisted workflow implements ${capability.alias} with ${matched}.`
					: `Persisted workflow is missing ${capability.alias}; expected one of ${capability.anyOf.join(", ")}.`,
				{
					path: "boards[].nodeTypes",
					expected: capability.anyOf.join(" | "),
					actual: matched ?? "missing",
				},
			),
		);
	}

	for (const reference of requirements.requiredIdReferences) {
		const source = reference.source ?? "canonical";
		const resolution = resolveEntity(
			reference.entity,
			reference.alias,
			snapshot,
		);
		const codeAlias = normalizeSemanticAlias(reference.alias);
		if (!resolution.entity || resolution.ambiguous) {
			checks.push(
				check(
					`flowscript.id_reference.${reference.entity}.${codeAlias}`,
					false,
					resolution.ambiguous
						? `${reference.entity} alias ${JSON.stringify(reference.alias)} is ambiguous.`
						: `Could not resolve ${reference.entity} alias ${JSON.stringify(reference.alias)}.`,
					{ expected: reference.alias, actual: "unresolved" },
				),
			);
			continue;
		}

		const referenced = sourceContainsReference(
			reference.entity,
			source,
			resolution.entity.id,
			authored,
			canonicalSource,
		);
		checks.push(
			check(
				`flowscript.id_reference.${reference.entity}.${codeAlias}`,
				referenced,
				referenced
					? `${source} FlowScript references ${reference.entity} id ${resolution.entity.id}.`
					: `${source} FlowScript does not reference resolved ${reference.entity} id ${resolution.entity.id}.`,
				{
					path: resolution.entity.path,
					expected: resolution.entity.id,
					actual: referenced,
				},
			),
		);
	}

	if (requirements.validateReferenceIntegrity) {
		checks.push(...integrityChecks(snapshot, boards, pages, widgets));
	}

	const failures = checks.filter((result) => result.status === "fail");
	return {
		schema: "flowpilot.app-creation-e2e-report/v1",
		caseId: caseDefinition.id,
		caseTitle: caseDefinition.title,
		appId: snapshot.appId,
		appName: snapshot.appName,
		expectedAppName: expectedName,
		model: snapshot.model ?? FLOWPILOT_E2E_DEFAULT_MODEL,
		passed: failures.length === 0,
		summary: {
			checks: checks.length,
			passed: checks.length - failures.length,
			failed: failures.length,
		},
		inventory: {
			boards: boards.length,
			totalNodes,
			pages: pages.length,
			widgets: widgets.length,
			tables: snapshot.tables.length,
			events: snapshot.events.length,
		},
		flowScript: {
			authored: authored ? flowScriptSizeMetrics(authored) : undefined,
			canonical: canonicalMetrics,
		},
		checks,
		failures,
	};
}
