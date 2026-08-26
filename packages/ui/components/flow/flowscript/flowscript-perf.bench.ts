/**
 * FlowScript language-tooling micro-benchmark.
 *
 * Run from packages/ui with:
 *   bun run components/flow/flowscript/flowscript-perf.bench.ts
 *
 * Builds the real catalog index from packages/ast/signatures.json plus the
 * generated names snapshot, synthesizes documents at ~500 / 2,000 / 5,000
 * lines from the committed AST fixtures, and times every per-keystroke code
 * path (median of warmed runs). "cold" = fresh text (analysis cache miss),
 * "warm" = same text again (cache hit).
 */

import { readFileSync } from "node:fs";
import type { Monaco } from "@monaco-editor/react";
import { loadFlowScriptNamesTable } from "../../../lib/flowscript/names";
import type { INode, IPin } from "../../../lib/schema/flow/node";
import {
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import {
	analyzeFlowScriptDocument,
	buildFlowScriptIndex,
	computeFlowScriptDiagnostics,
	registerFlowScriptProviders,
} from "./flowscript-language";

// ---------------------------------------------------------------------------
// Catalog from packages/ast/signatures.json
// ---------------------------------------------------------------------------

interface SignatureType {
	base: string;
	container: string;
}

interface SignaturePin {
	name: string;
	ty: SignatureType;
	optional: boolean;
	doc?: string;
	schema?: string;
}

interface Signature {
	node_type: string;
	display: string;
	friendly: string;
	category: string;
	inputs: SignaturePin[];
	outputs: SignaturePin[];
	impure: boolean;
	doc?: string;
	namespace?: string;
	alias?: string;
	receiver?: string;
}

const BASE_TYPES: Record<string, IVariableType> = {
	string: IVariableType.String,
	int: IVariableType.Integer,
	float: IVariableType.Float,
	bool: IVariableType.Boolean,
	Struct: IVariableType.Struct,
	Date: IVariableType.Date,
	Path: IVariableType.PathBuf,
	bytes: IVariableType.Byte,
	any: IVariableType.Generic,
};

const CONTAINERS: Record<string, IValueType> = {
	Normal: IValueType.Normal,
	Array: IValueType.Array,
	Set: IValueType.HashSet,
	Map: IValueType.HashMap,
};

function signaturePin(
	spec: SignaturePin,
	pinType: IPinType,
	index: number,
): IPin {
	return {
		id: `${pinType}-${spec.name}-${index}`,
		name: spec.name,
		friendly_name: spec.name,
		description: spec.doc ?? "",
		pin_type: pinType,
		data_type: BASE_TYPES[spec.ty.base] ?? IVariableType.Generic,
		value_type: CONTAINERS[spec.ty.container] ?? IValueType.Normal,
		index,
		connected_to: [],
		depends_on: [],
		default_value: spec.optional ? [1] : null,
		schema: spec.schema ?? null,
		options: null,
	};
}

function signatureNode(sig: Signature): INode {
	const pins: Record<string, IPin> = {};
	let index = 0;
	const add = (pin: IPin) => {
		pins[pin.id] = pin;
	};
	if (sig.impure) {
		add(
			signaturePin(
				{
					name: "exec_in",
					ty: { base: "exec", container: "Normal" },
					optional: false,
				},
				IPinType.Input,
				index++,
			),
		);
		pins[`${IPinType.Input}-exec_in-0`].data_type = IVariableType.Execution;
	}
	for (const input of sig.inputs)
		add(signaturePin(input, IPinType.Input, index++));
	if (sig.impure) {
		const exec = signaturePin(
			{
				name: "exec_out",
				ty: { base: "exec", container: "Normal" },
				optional: false,
			},
			IPinType.Output,
			index++,
		);
		exec.data_type = IVariableType.Execution;
		add(exec);
	}
	for (const output of sig.outputs)
		add(signaturePin(output, IPinType.Output, index++));
	return {
		id: sig.node_type,
		name: sig.node_type,
		friendly_name: sig.friendly,
		description: sig.doc ?? "",
		category: sig.category,
		pins,
	};
}

function loadCatalog(): INode[] {
	const url = new URL("../../../../ast/signatures.json", import.meta.url);
	const parsed = JSON.parse(readFileSync(url, "utf8")) as {
		signatures: Signature[];
	};
	return parsed.signatures.map(signatureNode);
}

// ---------------------------------------------------------------------------
// Documents from the committed fixtures
// ---------------------------------------------------------------------------

const FIXTURES = [
	"../../../../../tests/ast/bypaw6n2ksuvrw0kcaj14omz.anchored.flow",
	"../../../../../tests/ast/ttwctnp08u18sg2z6nmcqqak.anchored.flow",
];

/** Doc-local callable names in the fixtures, renamed per repetition so calls stay resolved. */
const LOCAL_NAMES =
	/\b(saveConfig|loadConfig|loadConfigDashboard|filterSources|perpareNews|notifyProjectUser|widgetActionEvent2?|writeReport|clickLatestOverview|loadOverview|generateFeed|simpleEvent|setupNewElement|showBriefing|setReportCreated)\b/g;
const LOCAL_VARS =
	/\b(database\d?|session\d?|rows\d?|historyOut\d?|elementRef\d?|date\d?|cuid\d?|item\d?|values|record|content|hash\d?|stats\d?|model\d?|arrayOut|index)\b/g;

function splitFixture(text: string): { header: string; body: string } {
	const lines = text.split("\n");
	let bodyStart = lines.length;
	for (let i = 0; i < lines.length; i++) {
		if (/^(function|@|[a-z][\w$]*\s+[\w$]+\s*\()/i.test(lines[i])) {
			if (/^(interface|struct|use|const|let)\b/.test(lines[i])) continue;
			bodyStart = i;
			break;
		}
	}
	return {
		header: lines.slice(0, bodyStart).join("\n"),
		body: lines.slice(bodyStart).join("\n"),
	};
}

function generateDocument(targetLines: number): string {
	const fixtures = FIXTURES.map((relative) =>
		readFileSync(new URL(relative, import.meta.url), "utf8"),
	);
	const parts = fixtures.map(splitFixture);
	const header = parts[0].header;
	const chunks: string[] = [header];
	let lineCount = header.split("\n").length;
	let repetition = 0;
	while (lineCount < targetLines) {
		const body = parts[repetition % parts.length].body;
		const suffixed = body
			.replace(LOCAL_NAMES, `$1R${repetition}`)
			.replace(LOCAL_VARS, `$1R${repetition}`);
		chunks.push(suffixed);
		lineCount += suffixed.split("\n").length;
		repetition++;
	}
	const text = chunks.join("\n");
	return text.split("\n").slice(0, targetLines).join("\n");
}

// ---------------------------------------------------------------------------
// Fake Monaco + model (mirrors the unit-test harness)
// ---------------------------------------------------------------------------

interface Pos {
	lineNumber: number;
	column: number;
}

function fakeMonaco(): Monaco {
	const disposable = { dispose: () => undefined };
	const register = () => disposable;
	return {
		MarkerSeverity: { Error: 8, Warning: 4 },
		editor: { getModelMarkers: () => [] },
		languages: {
			CompletionItemKind: {
				Property: 1,
				EnumMember: 2,
				Field: 3,
				Variable: 4,
				Function: 5,
				Interface: 6,
				Keyword: 7,
				TypeParameter: 8,
				Constant: 9,
				Method: 10,
				Module: 11,
				Snippet: 12,
			},
			CompletionItemInsertTextRule: { InsertAsSnippet: 1 },
			SymbolKind: {
				Namespace: 0,
				Interface: 1,
				Function: 2,
				Variable: 3,
				Event: 4,
			},
			FoldingRangeKind: { Imports: "imports" },
			InlayHintKind: { Type: 1, Parameter: 2 },
			registerCompletionItemProvider: register,
			registerHoverProvider: register,
			registerSignatureHelpProvider: register,
			registerCodeActionProvider: register,
			registerDocumentSymbolProvider: register,
			registerFoldingRangeProvider: register,
			registerInlayHintsProvider: register,
			registerDefinitionProvider: register,
			registerReferenceProvider: register,
			registerDocumentSemanticTokensProvider: register,
			registerRenameProvider: register,
		},
	} as unknown as Monaco;
}

interface CapturedProviders {
	completion: (
		model: unknown,
		position: Pos,
	) => { suggestions: unknown[] } | Promise<{ suggestions: unknown[] }>;
	semantic: (model: unknown) => unknown;
	inlay: (model: unknown, range: unknown) => unknown;
	folding: (model: unknown) => unknown;
	symbols: (model: unknown) => unknown;
}

function captureProviders(catalog: INode[]): CapturedProviders {
	const captured: Partial<CapturedProviders> = {};
	const monaco = fakeMonaco();
	const languages = monaco.languages as unknown as Record<string, unknown>;
	const keep =
		(key: keyof CapturedProviders, method: string) =>
		(_language: string, provider: Record<string, unknown>) => {
			if (!captured[key]) captured[key] = provider[method] as never;
			return { dispose: () => undefined };
		};
	languages.registerCompletionItemProvider = keep(
		"completion",
		"provideCompletionItems",
	);
	languages.registerDocumentSemanticTokensProvider = keep(
		"semantic",
		"provideDocumentSemanticTokens",
	);
	languages.registerInlayHintsProvider = keep("inlay", "provideInlayHints");
	languages.registerFoldingRangeProvider = keep(
		"folding",
		"provideFoldingRanges",
	);
	languages.registerDocumentSymbolProvider = keep(
		"symbols",
		"provideDocumentSymbols",
	);
	registerFlowScriptProviders(monaco, () => catalog);
	return captured as CapturedProviders;
}

function offsetAt(text: string, position: Pos): number {
	const lines = text.split("\n");
	return (
		lines
			.slice(0, position.lineNumber - 1)
			.reduce((sum, line) => sum + line.length + 1, 0) +
		position.column -
		1
	);
}

function endPosition(text: string): Pos {
	const lines = text.split("\n");
	return { lineNumber: lines.length, column: (lines.at(-1) ?? "").length + 1 };
}

let versionCounter = 1;

function model(text: string) {
	const version = versionCounter++;
	return {
		uri: {},
		getValue: () => text,
		getVersionId: () => version,
		getOffsetAt: (position: Pos) => offsetAt(text, position),
		getWordUntilPosition: (position: Pos) => {
			const line = text.split("\n")[position.lineNumber - 1] ?? "";
			let start = position.column - 1;
			while (start > 0 && /[\w$]/.test(line[start - 1])) start--;
			return {
				word: line.slice(start, position.column - 1),
				startColumn: start + 1,
				endColumn: position.column,
			};
		},
	};
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

const RUNS = 7;
const WARMUP = 2;

async function median(fn: () => unknown | Promise<unknown>): Promise<number> {
	const samples: number[] = [];
	for (let i = 0; i < WARMUP + RUNS; i++) {
		const start = performance.now();
		await fn();
		const elapsed = performance.now() - start;
		if (i >= WARMUP) samples.push(elapsed);
	}
	samples.sort((a, b) => a - b);
	return samples[Math.floor(samples.length / 2)];
}

function formatMs(ms: number): string {
	return ms >= 10 ? ms.toFixed(1) : ms.toFixed(2);
}

interface Row {
	item: string;
	cold: number;
	warm?: number;
}

async function benchSize(
	label: string,
	targetLines: number,
	catalog: INode[],
): Promise<Row[]> {
	const base = generateDocument(targetLines);
	const index = buildFlowScriptIndex(catalog, await loadFlowScriptNamesTable());
	const providers = captureProviders(catalog);
	const monaco = fakeMonaco();
	let unique = 0;
	const fresh = () => `${base}\n// v${unique++}`;
	const rows: Row[] = [];

	rows.push({
		item: "analyzeFlowScriptDocument",
		cold: await median(() => analyzeFlowScriptDocument(fresh(), index)),
		warm: await (async () => {
			const text = fresh();
			analyzeFlowScriptDocument(text, index);
			return median(() => analyzeFlowScriptDocument(text, index));
		})(),
	});

	rows.push({
		item: "computeFlowScriptDiagnostics",
		cold: await median(() =>
			computeFlowScriptDiagnostics(monaco, fresh(), catalog),
		),
	});

	rows.push({
		item: "semantic tokens (full)",
		cold: await median(() => providers.semantic(model(fresh()))),
		warm: await (async () => {
			const m = model(fresh());
			providers.semantic(m);
			return median(() => providers.semantic(m));
		})(),
	});

	const completionAt = async (tail: string) => {
		return median(async () => {
			const text = `${base}\n// v${unique++}\n${tail}`;
			const position = endPosition(text);
			await providers.completion(model(text), position);
		});
	};
	rows.push({ item: "completion (bare)", cold: await completionAt("log") });
	rows.push({
		item: "completion (after . )",
		cold: await completionAt('const recvX: string = "abc"\nrecvX.'),
	});

	rows.push({
		item: "inlay hints (60 lines)",
		cold: await median(() =>
			providers.inlay(model(fresh()), {
				startLineNumber: 1,
				startColumn: 1,
				endLineNumber: 60,
				endColumn: 1,
			}),
		),
		warm: await (async () => {
			const m = model(fresh());
			const range = {
				startLineNumber: 1,
				startColumn: 1,
				endLineNumber: 60,
				endColumn: 1,
			};
			providers.inlay(m, range);
			return median(() => providers.inlay(m, range));
		})(),
	});

	rows.push({
		item: "folding",
		cold: await median(() => providers.folding(model(fresh()))),
		warm: await (async () => {
			const m = model(fresh());
			providers.folding(m);
			return median(() => providers.folding(m));
		})(),
	});

	rows.push({
		item: "document symbols",
		cold: await median(() => providers.symbols(model(fresh()))),
		warm: await (async () => {
			const m = model(fresh());
			providers.symbols(m);
			return median(() => providers.symbols(m));
		})(),
	});

	rows.push({
		item: "parseFlowScriptAnchors",
		cold: await median(() => parseFlowScriptAnchors(fresh())),
	});

	console.log(
		`\n### ${label} (${base.split("\n").length} lines, ${(base.length / 1024).toFixed(0)} KB)`,
	);
	console.log(
		`${"item".padEnd(30)} ${"cold ms".padStart(9)} ${"warm ms".padStart(9)}`,
	);
	for (const row of rows) {
		console.log(
			`${row.item.padEnd(30)} ${formatMs(row.cold).padStart(9)} ${
				row.warm !== undefined ? formatMs(row.warm).padStart(9) : "".padStart(9)
			}`,
		);
	}
	return rows;
}

async function main(): Promise<void> {
	const catalog = loadCatalog();
	const names = await loadFlowScriptNamesTable();
	const buildStart = performance.now();
	const index = buildFlowScriptIndex(catalog, names);
	console.log(
		`catalog: ${catalog.length} nodes, index built in ${formatMs(performance.now() - buildStart)} ms (${index.byName.size} flat names, ${index.byQualified.size} qualified)`,
	);
	await benchSize("~500 lines", 500, catalog);
	await benchSize("~2,000 lines", 2000, catalog);
	await benchSize("~5,000 lines", 5000, catalog);
	await benchWorker(catalog);
}

await main();

// ---------------------------------------------------------------------------
// Worker round-trip (real Worker; bun supports the Web Worker API)
// ---------------------------------------------------------------------------

interface WorkerHarness {
	worker: Worker;
	request: (message: Record<string, unknown>) => Promise<unknown>;
	post: (message: Record<string, unknown>) => void;
}

function createWorkerHarness(): WorkerHarness {
	const worker = new Worker(
		new URL("./flowscript-language.worker.ts", import.meta.url),
		{ type: "module" },
	);
	const pendingRequests = new Map<number, (value: unknown) => void>();
	worker.onmessage = (event: MessageEvent<{ id: number }>) => {
		const resolve = pendingRequests.get(event.data.id);
		if (resolve) {
			pendingRequests.delete(event.data.id);
			resolve(event.data);
		}
	};
	return {
		worker,
		post: (message) => worker.postMessage(message),
		request: (message) =>
			new Promise((resolve) => {
				pendingRequests.set(message.id as number, resolve);
				worker.postMessage(message);
			}),
	};
}

async function benchWorker(catalog: INode[]): Promise<void> {
	console.log("\n### Worker round-trip (real Worker, ~5,000-line doc)");
	let harness: WorkerHarness;
	try {
		harness = createWorkerHarness();
	} catch (error) {
		console.log(`worker unavailable in this runtime: ${error}`);
		return;
	}
	const names = await loadFlowScriptNamesTable();
	const initStart = performance.now();
	harness.post({ kind: "init-catalog", catalogId: 1, nodes: catalog, names });
	const initSend = performance.now() - initStart;
	const doc = generateDocument(5000);
	let id = 0;
	let version = 0;
	const roundTrip = async (
		kind: string,
		withText: boolean,
		extra: Record<string, unknown> = {},
	) => {
		if (withText) version++;
		return median(async () => {
			await harness.request({
				kind,
				id: ++id,
				catalogId: 1,
				doc: {
					uri: "bench",
					version,
					text: withText ? `${doc}\n// v${version}` : undefined,
				},
				...extra,
			});
			if (withText) version++;
		});
	};
	console.log(
		`init-catalog postMessage (send side, ${catalog.length} nodes): ${formatMs(initSend)} ms`,
	);
	console.log(
		`diagnostics cold (text crosses, analysis cold): ${formatMs(await roundTrip("diagnostics", true))} ms`,
	);
	console.log(
		`semantic-tokens warm (doc + analysis cached):   ${formatMs(await roundTrip("semantic-tokens", false))} ms`,
	);
	console.log(
		`folding warm (doc + analysis cached):           ${formatMs(await roundTrip("folding", false))} ms`,
	);
	console.log(
		`env-snapshot warm (doc + analysis cached):      ${formatMs(await roundTrip("env-snapshot", false))} ms`,
	);
	// Main-thread blocking share of one keystroke: the postMessage that ships the text.
	const sendOnly = await median(() => {
		version++;
		harness.post({
			kind: "diagnostics",
			id: ++id,
			catalogId: 1,
			doc: { uri: "bench", version, text: `${doc}\n// v${version}` },
		});
	});
	console.log(
		`postMessage of 735 KB doc (main-thread blocking): ${formatMs(sendOnly)} ms`,
	);
	harness.worker.terminate();
}
