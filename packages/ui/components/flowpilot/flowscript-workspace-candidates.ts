import {
	type FlowScriptNamesTable,
	getFlowScriptNamesTable,
	loadFlowScriptNamesTable,
} from "../../lib/flowscript/names";

export interface FlowScriptWorkspaceCandidate {
	source: string;
	status?: string;
	completion?: string;
	retained_full_source?: string;
	regression?: Record<string, unknown>;
	/** Bounded legacy compiler diagnostics carried with this exact source snapshot. */
	diagnostics?: unknown[];
	/** Bounded structured compiler diagnostics carried with this exact source snapshot. */
	structured_diagnostics?: unknown[];
}

export interface FlowScriptCandidateProfile {
	callSites: number;
	meaningfulStatements: number;
	eventEntries: number;
	helperFunctions: string[];
	nonEmptyHelperFunctions: string[];
	topLevelVariables: string[];
	interfaces: string[];
	callNames: string[];
	eventsCallingHelpers: number;
	helperDomainCallSites: number;
}

export interface FlowScriptCandidateRegression {
	previous_call_sites: number;
	candidate_call_sites: number;
	previous_statements: number;
	candidate_statements: number;
	previous_scope_symbols: number;
	retained_scope_symbols: number;
}

/**
 * Calls that do not count as domain work. Spelled as node ids (the normalised form once the
 * names snapshot is loaded) plus every legacy flat, qualified and method spelling so the
 * heuristic keeps working before the snapshot arrives.
 */
const TRIVIAL_SMOKE_CALLS = new Set([
	"log",
	"loginfo",
	"logdebug",
	"logwarn",
	"logerror",
	"printinfo",
	"printdebug",
	"printwarn",
	"printerror",
	"stringformat",
	"structmake",
	"structget",
	"structset",
	"arraypush",
	"arrayget",
	"arraylength",
	"variableget",
	"log_info",
	"log_debug",
	"log_warn",
	"log_error",
	"string_format",
	"struct_make",
	"struct_get",
	"struct_set",
	"array_push",
	"array_get",
	"array_length",
	"variable_get",
	"log::info",
	"log::debug",
	"log::warn",
	"log::error",
	"string::format",
	"struct::make",
	"struct::get",
	"struct::set",
	"array::push",
	"array::get",
	"array::length",
	"variable::get",
	".format",
	".get",
	".set",
	".push",
	".length",
]);

const CONTROL_CALL_NAMES = new Set(["if", "for", "while", "switch"]);

/**
 * Opening line of a `detached { … }` container. Every other block header names a board object
 * this profile records separately; this one has no node behind it — hence no anchor — so it is
 * punctuation like a bare brace and only the chain inside it counts as workflow.
 */
const DETACHED_CONTAINER_LINE = /^detached\s*\{$/;

function normalizedSymbol(value: string) {
	return value.trim().toLowerCase();
}

function braceDelta(line: string) {
	return (line.match(/{/g)?.length ?? 0) - (line.match(/}/g)?.length ?? 0);
}

interface CallNameIndex {
	byFlat: Map<string, string>;
	byQualified: Map<string, string>;
	/** alias → node ids callable in method form (`x.alias()`). */
	byAlias: Map<string, string[]>;
}

let cachedNamesTable: FlowScriptNamesTable | undefined;
let cachedCallNameIndex: CallNameIndex | undefined;

function callNameIndex(): CallNameIndex | undefined {
	const table = getFlowScriptNamesTable();
	if (!table) {
		loadFlowScriptNamesTable().catch(() => undefined);
		return undefined;
	}
	if (table === cachedNamesTable && cachedCallNameIndex)
		return cachedCallNameIndex;
	const index: CallNameIndex = {
		byFlat: new Map(),
		byQualified: new Map(),
		byAlias: new Map(),
	};
	for (const [nodeType, names] of Object.entries(table)) {
		index.byFlat.set(normalizedSymbol(names.flat), nodeType);
		index.byQualified.set(normalizedSymbol(names.qualified), nodeType);
		if (names.receiver) {
			const alias = normalizedSymbol(names.alias);
			index.byAlias.set(alias, [...(index.byAlias.get(alias) ?? []), nodeType]);
		}
	}
	cachedNamesTable = table;
	cachedCallNameIndex = index;
	return index;
}

const CALL_HEAD_RE =
	/(\.)?\s*((?:[A-Za-z_][A-Za-z0-9_]*\s*::\s*)*[A-Za-z_][A-Za-z0-9_]*)\s*\(/g;

/**
 * Normalises one call head to a node id via the names snapshot when possible: flat
 * (`logInfo`), qualified (`log::info`) and method (`s.format()`) spellings of a node collapse
 * to the same key. Unknown names (user functions, unloaded snapshot) keep their lowercased
 * spelling; method calls keep a leading `.`.
 */
export function normalizeCallName(
	spelling: string,
	method: boolean,
	helpers?: ReadonlySet<string>,
): string {
	const name = normalizedSymbol(spelling).replace(/\s*::\s*/g, "::");
	const index = callNameIndex();
	if (method) {
		if (helpers?.has(name)) return name;
		const candidates = index?.byAlias.get(name);
		return candidates?.length === 1 ? candidates[0] : `.${name}`;
	}
	if (name.includes("::")) return index?.byQualified.get(name) ?? name;
	if (helpers?.has(name)) return name;
	return index?.byFlat.get(name) ?? name;
}

function callNamesInLine(line: string, helpers?: ReadonlySet<string>) {
	return [...line.matchAll(CALL_HEAD_RE)]
		.map((match) =>
			normalizeCallName(match[2] ?? "", match[1] === ".", helpers),
		)
		.filter((name) => name && !CONTROL_CALL_NAMES.has(name));
}

/** Lightweight structural profile used as a frontend safety net across provider implementations. */
export function profileFlowScriptCandidate(
	source: string,
): FlowScriptCandidateProfile {
	const helperFunctions = new Set<string>();
	for (const match of source.matchAll(
		/\bfunction\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/g,
	)) {
		helperFunctions.add(normalizedSymbol(match[1] ?? ""));
	}

	const nonEmptyHelperFunctions = new Set<string>();
	const topLevelVariables = new Set<string>();
	const interfaces = new Set<string>();
	const callNames = new Set<string>();
	let callSites = 0;
	let meaningfulStatements = 0;
	let eventEntries = 0;
	let eventsCallingHelpers = 0;
	let helperDomainCallSites = 0;
	let depth = 0;
	let activeHelper: { name: string; depth: number } | undefined;
	let activeEvent: { calledHelper: boolean; depth: number } | undefined;

	for (const rawLine of source.replace(/\r\n/g, "\n").split("\n")) {
		const line = rawLine.trim();
		if (
			!line ||
			line === "{" ||
			line === "}" ||
			line.startsWith("//") ||
			line.startsWith("@") ||
			DETACHED_CONTAINER_LINE.test(line)
		) {
			depth += braceDelta(rawLine);
			if (activeHelper && depth < activeHelper.depth) activeHelper = undefined;
			if (activeEvent && depth < activeEvent.depth) activeEvent = undefined;
			continue;
		}
		meaningfulStatements += 1;

		const helperDeclaration = line.match(
			/^function\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/,
		);
		const eventDeclaration = line.match(
			/^(events[A-Za-z0-9_]*)(?:\s+([A-Za-z_][A-Za-z0-9_]*))?\s*\(/,
		);
		const interfaceDeclaration = line.match(
			/^interface\s+([A-Za-z_][A-Za-z0-9_]*)/,
		);
		if (depth === 0) {
			const variable = line.match(/^(?:const|let)\s+([A-Za-z_][A-Za-z0-9_]*)/);
			if (variable?.[1]) topLevelVariables.add(normalizedSymbol(variable[1]));
		}
		if (interfaceDeclaration?.[1]) {
			interfaces.add(normalizedSymbol(interfaceDeclaration[1]));
		}
		if (helperDeclaration?.[1]) {
			activeHelper = {
				name: normalizedSymbol(helperDeclaration[1]),
				depth: depth + Math.max(1, braceDelta(rawLine)),
			};
		}
		if (eventDeclaration?.[1]) {
			eventEntries += 1;
			activeEvent = {
				calledHelper: false,
				depth: depth + Math.max(1, braceDelta(rawLine)),
			};
		}

		const helperDeclarationName = normalizedSymbol(
			helperDeclaration?.[1] ?? "",
		);
		const eventDeclarationType = normalizeCallName(
			eventDeclaration?.[1] ?? "",
			false,
		);
		const eventDeclarationAlias = normalizedSymbol(eventDeclaration?.[2] ?? "");
		const calls = callNamesInLine(line, helperFunctions).filter(
			(name) =>
				name !== helperDeclarationName &&
				name !== eventDeclarationType &&
				name !== eventDeclarationAlias,
		);
		for (const name of calls) {
			callSites += 1;
			callNames.add(name);
			if (activeHelper) {
				nonEmptyHelperFunctions.add(activeHelper.name);
				if (!helperFunctions.has(name) && !TRIVIAL_SMOKE_CALLS.has(name)) {
					helperDomainCallSites += 1;
				}
			}
			if (
				activeEvent &&
				helperFunctions.has(name) &&
				!activeEvent.calledHelper
			) {
				activeEvent.calledHelper = true;
				eventsCallingHelpers += 1;
			}
		}

		depth += braceDelta(rawLine);
		if (activeHelper && depth < activeHelper.depth) activeHelper = undefined;
		if (activeEvent && depth < activeEvent.depth) activeEvent = undefined;
	}

	return {
		callSites,
		meaningfulStatements,
		eventEntries,
		helperFunctions: [...helperFunctions].sort(),
		nonEmptyHelperFunctions: [...nonEmptyHelperFunctions].sort(),
		topLevelVariables: [...topLevelVariables].sort(),
		interfaces: [...interfaces].sort(),
		callNames: [...callNames].sort(),
		eventsCallingHelpers,
		helperDomainCallSites,
	};
}

function profileScore(profile: FlowScriptCandidateProfile) {
	return (
		profile.callSites * 8 +
		profile.meaningfulStatements * 2 +
		profile.helperFunctions.length * 12 +
		profile.eventEntries * 6 +
		profile.topLevelVariables.length * 4 +
		profile.interfaces.length * 4 +
		profile.callNames.length * 2
	);
}

function stableScopeSymbols(profile: FlowScriptCandidateProfile) {
	return new Set([
		...profile.helperFunctions.map((name) => `function:${name}`),
		...profile.topLevelVariables.map((name) => `variable:${name}`),
		...profile.interfaces.map((name) => `interface:${name}`),
		...(profile.eventEntries > 0 ? ["event:present"] : []),
	]);
}

export function detectFlowScriptCandidateRegression(
	previous: FlowScriptCandidateProfile,
	candidate: FlowScriptCandidateProfile,
): FlowScriptCandidateRegression | undefined {
	const previousIsSubstantial =
		previous.callSites >= 5 &&
		(previous.meaningfulStatements >= 6 ||
			previous.helperFunctions.length + previous.eventEntries >= 3);
	const severeCallShrink = candidate.callSites * 3 < previous.callSites;
	const severeStatementShrink =
		candidate.meaningfulStatements * 2 < previous.meaningfulStatements;
	if (!(previousIsSubstantial && severeCallShrink && severeStatementShrink)) {
		return undefined;
	}

	const previousSymbols = stableScopeSymbols(previous);
	const candidateSymbols = stableScopeSymbols(candidate);
	const retainedScopeSymbols = [...previousSymbols].filter((symbol) =>
		candidateSymbols.has(symbol),
	).length;
	const identityWasLost =
		previousSymbols.size >= 2 &&
		retainedScopeSymbols * 2 < previousSymbols.size;
	const multipleEventScopeWasLost =
		previous.eventEntries >= 2 &&
		candidate.eventEntries * 2 < previous.eventEntries;
	if (!(identityWasLost || multipleEventScopeWasLost)) return undefined;

	return {
		previous_call_sites: previous.callSites,
		candidate_call_sites: candidate.callSites,
		previous_statements: previous.meaningfulStatements,
		candidate_statements: candidate.meaningfulStatements,
		previous_scope_symbols: previousSymbols.size,
		retained_scope_symbols: retainedScopeSymbols,
	};
}

function isModularWorkingSlice(profile: FlowScriptCandidateProfile) {
	return (
		profile.nonEmptyHelperFunctions.length > 0 &&
		profile.eventsCallingHelpers > 0 &&
		profile.helperDomainCallSites > 0
	);
}

function sourceKey(source: string): string {
	return source.replace(/\r\n/g, "\n").trim();
}

const MAX_WORKSPACE_DIAGNOSTICS = 20;
const MAX_WORKSPACE_DIAGNOSTIC_TEXT_CHARS = 600;
const WORKSPACE_DIAGNOSTIC_FIELDS = [
	"id",
	"code",
	"phase",
	"severity",
	"message",
	"line",
	"column",
	"span",
	"source_span",
	"path",
	"ast_path",
	"scope",
	"function",
	"expected",
	"actual",
	"declaration",
	"pin",
	"fix",
	"occurrences",
	"related_messages",
] as const;

function boundedWorkspaceDiagnostic(value: unknown): unknown {
	if (typeof value === "string") {
		return value.slice(0, MAX_WORKSPACE_DIAGNOSTIC_TEXT_CHARS);
	}
	if (
		value === null ||
		typeof value === "number" ||
		typeof value === "boolean"
	) {
		return value;
	}
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		return String(value).slice(0, MAX_WORKSPACE_DIAGNOSTIC_TEXT_CHARS);
	}

	const record = value as Record<string, unknown>;
	const bounded: Record<string, unknown> = {};
	for (const key of WORKSPACE_DIAGNOSTIC_FIELDS) {
		const field = record[key];
		if (field === undefined) continue;
		if (
			field === null ||
			typeof field === "number" ||
			typeof field === "boolean"
		) {
			bounded[key] = field;
			continue;
		}
		if (typeof field === "string") {
			bounded[key] = field.slice(0, MAX_WORKSPACE_DIAGNOSTIC_TEXT_CHARS);
			continue;
		}
		try {
			bounded[key] = JSON.stringify(field).slice(
				0,
				MAX_WORKSPACE_DIAGNOSTIC_TEXT_CHARS,
			);
		} catch {
			bounded[key] = "[diagnostic detail unavailable]";
		}
	}
	return Object.keys(bounded).length > 0
		? bounded
		: "[unstructured diagnostic omitted]";
}

function boundedWorkspaceDiagnostics(value: unknown): unknown[] | undefined {
	if (!Array.isArray(value) || value.length === 0) return undefined;
	return value
		.slice(0, MAX_WORKSPACE_DIAGNOSTICS)
		.map(boundedWorkspaceDiagnostic);
}

/** Prefer the structured compiler list while retaining compatibility with legacy diagnostics. */
export function flowScriptWorkspaceDiagnostics(
	candidate: FlowScriptWorkspaceCandidate,
): readonly unknown[] {
	return candidate.structured_diagnostics?.length
		? candidate.structured_diagnostics
		: (candidate.diagnostics ?? []);
}

/** A later clean candidate supersedes earlier transient validation-error snapshots. */
export function flowScriptWorkspaceRepairResolved(
	candidate: FlowScriptWorkspaceCandidate,
): boolean {
	return [
		"valid",
		"queued",
		"already_queued",
		"no_changes",
		"applied",
	].includes(candidate.status?.trim().toLowerCase() ?? "");
}

/** Parse either a raw FlowScript document or the streamed `{source,status}` envelope. */
export function parseFlowScriptWorkspaceCandidate(
	workspace: string | undefined,
): FlowScriptWorkspaceCandidate | undefined {
	const trimmed = workspace?.trim();
	if (!trimmed) return undefined;
	try {
		const parsed = JSON.parse(trimmed);
		if (typeof parsed === "string" && parsed.trim()) {
			return { source: parsed };
		}
		if (
			parsed &&
			typeof parsed === "object" &&
			typeof parsed.source === "string" &&
			parsed.source.trim()
		) {
			const candidate: FlowScriptWorkspaceCandidate = {
				source: parsed.source,
			};
			if (typeof parsed.status === "string") candidate.status = parsed.status;
			if (typeof parsed.completion === "string")
				candidate.completion = parsed.completion;
			if (typeof parsed.retained_full_source === "string")
				candidate.retained_full_source = parsed.retained_full_source;
			if (
				parsed.regression &&
				typeof parsed.regression === "object" &&
				!Array.isArray(parsed.regression)
			) {
				candidate.regression = parsed.regression as Record<string, unknown>;
			}
			const diagnostics = boundedWorkspaceDiagnostics(parsed.diagnostics);
			if (diagnostics) candidate.diagnostics = diagnostics;
			const structuredDiagnostics = boundedWorkspaceDiagnostics(
				parsed.structured_diagnostics,
			);
			if (structuredDiagnostics) {
				candidate.structured_diagnostics = structuredDiagnostics;
			}
			return candidate;
		}
	} catch {
		// A plain FlowScript document is the common final-response shape.
	}
	return { source: trimmed };
}

/** Extract every workspace frame from one transport chunk, preserving order. */
export function extractFlowScriptWorkspaceCandidates(text: string): {
	candidates: FlowScriptWorkspaceCandidate[];
	remainder: string;
} {
	const pattern = /<flowscript_workspace>([\s\S]*?)<\/flowscript_workspace>/g;
	const candidates: FlowScriptWorkspaceCandidate[] = [];
	for (const match of text.matchAll(pattern)) {
		const candidate = parseFlowScriptWorkspaceCandidate(match[1]);
		if (candidate) candidates.push(candidate);
	}
	return {
		candidates,
		remainder: text.replace(pattern, ""),
	};
}

/** Keep a bounded per-turn history so source and validation status remain an atomic pair. */
export function rememberFlowScriptWorkspaceCandidate(
	history: readonly FlowScriptWorkspaceCandidate[],
	candidate: FlowScriptWorkspaceCandidate,
): FlowScriptWorkspaceCandidate[] {
	if (!candidate.source.trim()) return [...history];
	const next = [...history, candidate];
	if (next.length <= 30) return next;
	const best = selectBestRecoverableFlowScriptCandidate(next);
	const recent = next.slice(-29);
	return best && !recent.some((entry) => entry === best)
		? [best, ...recent]
		: next.slice(-30);
}

/** Select the structurally closest retained draft, preferring the latest candidate on score ties. */
export function selectBestRecoverableFlowScriptCandidate(
	history: readonly FlowScriptWorkspaceCandidate[],
): FlowScriptWorkspaceCandidate | undefined {
	let best: FlowScriptWorkspaceCandidate | undefined;
	let bestScore = -1;
	for (const candidate of history) {
		if (!candidate.source.trim()) continue;
		const score = profileScore(profileFlowScriptCandidate(candidate.source));
		if (score >= bestScore) {
			best = candidate;
			bestScore = score;
		}
	}
	return best;
}

/**
 * Frontend backstop for providers that do not run the core repair tracker. A queued smoke-test
 * replacement is downgraded to the retained repair draft. A genuine modular working slice stays
 * applicable, but is explicitly marked partial and keeps the fuller source for continuation.
 */
export function protectFlowScriptCandidateCompleteness(
	history: readonly FlowScriptWorkspaceCandidate[],
	candidate: FlowScriptWorkspaceCandidate | undefined,
): FlowScriptWorkspaceCandidate | undefined {
	if (!candidate || candidate.status !== "queued") return candidate;
	const candidateKey = sourceKey(candidate.source);
	const previous = selectBestRecoverableFlowScriptCandidate(
		history.filter((entry) => sourceKey(entry.source) !== candidateKey),
	);
	if (!previous) return candidate;
	const previousProfile = profileFlowScriptCandidate(previous.source);
	const candidateProfile = profileFlowScriptCandidate(candidate.source);
	const regression = detectFlowScriptCandidateRegression(
		previousProfile,
		candidateProfile,
	);
	if (!regression) return candidate;
	if (
		candidate.completion === "partial_working_slice" ||
		isModularWorkingSlice(candidateProfile)
	) {
		return {
			...candidate,
			completion: "partial_working_slice",
			retained_full_source: candidate.retained_full_source ?? previous.source,
			regression: candidate.regression ?? { ...regression },
		};
	}
	return {
		...previous,
		status: "validation_errors",
		completion: "regression_blocked",
		retained_full_source: previous.source,
		regression: {
			...regression,
			code: "candidate_regression",
			submitted_status: candidate.status,
		},
	};
}

/**
 * Resolve a final raw workspace against the status last reported for that exact
 * source. Never borrow a status from a different candidate.
 */
export function resolveFlowScriptWorkspaceCandidate(
	history: readonly FlowScriptWorkspaceCandidate[],
	finalCandidate?: FlowScriptWorkspaceCandidate,
): FlowScriptWorkspaceCandidate | undefined {
	if (!finalCandidate) return history.at(-1);
	if (finalCandidate.status) return finalCandidate;

	const key = sourceKey(finalCandidate.source);
	for (let index = history.length - 1; index >= 0; index--) {
		const candidate = history[index];
		if (sourceKey(candidate.source) === key && candidate.status) {
			return {
				...candidate,
				...finalCandidate,
				source: finalCandidate.source,
				status: candidate.status,
			};
		}
	}
	return finalCandidate;
}

/**
 * Resolve the final backend field. Some external-agent transports return the
 * validated source as a raw string and no workspace stream frames; a non-empty
 * validated command batch is the only safe evidence that this raw source was
 * actually queued.
 */
export function resolveFinalFlowScriptWorkspaceCandidate(
	history: readonly FlowScriptWorkspaceCandidate[],
	workspace: string | undefined,
	hasValidatedCommands: boolean,
): FlowScriptWorkspaceCandidate | undefined {
	const parsed = parseFlowScriptWorkspaceCandidate(workspace);
	const resolved = resolveFlowScriptWorkspaceCandidate(history, parsed);
	const validated =
		resolved && parsed && !parsed.status && hasValidatedCommands
			? { ...resolved, status: "queued" }
			: resolved;
	return protectFlowScriptCandidateCompleteness(history, validated);
}

/** Only an explicitly queued, non-empty candidate can enter the apply path. */
export function isFlowScriptWorkspaceApplicable(
	candidate: FlowScriptWorkspaceCandidate | undefined,
): boolean {
	return candidate?.status === "queued" && Boolean(candidate.source.trim());
}

export function isPartialFlowScriptWorkspace(
	candidate: FlowScriptWorkspaceCandidate | undefined,
): boolean {
	return candidate?.completion === "partial_working_slice";
}

/** Partial slices may run on the board, but must never become app-level Events. */
export function shouldPromoteFlowScriptWorkspaceEvents(
	candidate: FlowScriptWorkspaceCandidate | undefined,
	applyFailed: boolean,
	hasAppliedWork: boolean,
): boolean {
	return (
		!applyFailed && hasAppliedWork && !isPartialFlowScriptWorkspace(candidate)
	);
}
