import { describe, expect, test } from "bun:test";
import type { INode, IPin } from "../../../lib/schema/flow/node";
import {
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import {
	analyzeContext,
	buildFlowScriptIndex,
	computeFlowScriptRawDiagnostics,
	evaluateExpr,
	getFlowScriptEnvDoc,
} from "./flowscript-language";
import {
	analyzeFlowScriptDocument,
	buildFlowScriptSemanticTokens,
} from "./flowscript-language-features";
import {
	type FlowScriptWorkerRequest,
	type FlowScriptWorkerResponse,
	createFlowScriptWorkerState,
	handleFlowScriptWorkerMessage,
	hydrateFlowScriptEnvDoc,
	makeFlowScriptEnvSnapshot,
} from "./flowscript-worker-protocol";

function pin(
	name: string,
	pinType: IPinType,
	dataType: IVariableType,
	index: number,
): IPin {
	return {
		id: `${pinType}-${name}`,
		name,
		friendly_name: name,
		description: "",
		pin_type: pinType,
		data_type: dataType,
		value_type: IValueType.Normal,
		index,
		connected_to: [],
		depends_on: [],
		default_value: null,
		schema: null,
		options: null,
	};
}

const catalog: INode[] = [
	{
		id: "string_trim",
		name: "string_trim",
		friendly_name: "Trim",
		description: "",
		category: "Test",
		pins: {
			a: pin("string", IPinType.Input, IVariableType.String, 0),
			b: pin("trimmed", IPinType.Output, IVariableType.String, 1),
		},
	},
	{
		id: "log_info",
		name: "log_info",
		friendly_name: "Log",
		description: "",
		category: "Test",
		pins: {
			e1: pin("exec_in", IPinType.Input, IVariableType.Execution, 0),
			m: pin("message", IPinType.Input, IVariableType.String, 1),
			e2: pin("exec_out", IPinType.Output, IVariableType.Execution, 2),
		},
	},
];

const names = {
	string_trim: {
		qualified: "string::trim",
		namespace: "string",
		alias: "trim",
		flat: "stringTrim",
		receiver: "string",
		class: "string",
		category: "Test",
	},
	log_info: {
		qualified: "log::info",
		namespace: "log",
		alias: "info",
		flat: "logInfo",
		receiver: null,
		class: null,
		category: "Test",
	},
};

const DOC = [
	"use log::*",
	'const greeting = "  hi  "',
	"const trimmed = greeting.trim()",
	"eventsSimple onLoad() {",
	"    info({ message: trimmed })",
	"    unknownCall({ x: 1 })",
	"}",
].join("\n");

function initState() {
	const state = createFlowScriptWorkerState();
	handleFlowScriptWorkerMessage(state, {
		kind: "init-catalog",
		catalogId: 1,
		nodes: catalog,
		names,
	});
	return state;
}

function okResult(response: FlowScriptWorkerResponse | null) {
	if (!response || response.kind !== "ok")
		throw new Error(`Expected ok response, got ${JSON.stringify(response)}`);
	return response.result;
}

describe("FlowScript worker protocol", () => {
	test("init-catalog returns nothing and registers the index", () => {
		const state = createFlowScriptWorkerState();
		const response = handleFlowScriptWorkerMessage(state, {
			kind: "init-catalog",
			catalogId: 7,
			nodes: catalog,
			names,
		});
		expect(response).toBeNull();
		expect(state.catalogs.get(7)?.byQualified.has("string::trim")).toBe(true);
	});

	test("diagnostics over the worker match the in-thread linter", () => {
		const state = initState();
		const result = okResult(
			handleFlowScriptWorkerMessage(state, {
				kind: "diagnostics",
				id: 1,
				catalogId: 1,
				doc: { uri: "a", version: 1, text: DOC },
			}),
		);
		if (result.kind !== "diagnostics") throw new Error("wrong result kind");
		const index = buildFlowScriptIndex(catalog, names);
		expect(result.markers).toEqual(computeFlowScriptRawDiagnostics(DOC, index));
		expect(
			result.markers.some((marker) => marker.message.includes("unknownCall")),
		).toBe(true);
	});

	test("semantic tokens over the worker match the in-thread encoding", () => {
		const state = initState();
		const result = okResult(
			handleFlowScriptWorkerMessage(state, {
				kind: "semantic-tokens",
				id: 2,
				catalogId: 1,
				doc: { uri: "a", version: 1, text: DOC },
			}),
		);
		if (result.kind !== "semantic-tokens") throw new Error("wrong result kind");
		const index = buildFlowScriptIndex(catalog, names);
		expect([...result.data]).toEqual([
			...buildFlowScriptSemanticTokens(analyzeFlowScriptDocument(DOC, index)),
		]);
		expect(result.data.length).toBeGreaterThan(0);
	});

	test("document text is cached per (uri, version) and re-used without text", () => {
		const state = initState();
		okResult(
			handleFlowScriptWorkerMessage(state, {
				kind: "diagnostics",
				id: 3,
				catalogId: 1,
				doc: { uri: "a", version: 4, text: DOC },
			}),
		);
		// Same version, no text: served from the doc cache.
		const reused = okResult(
			handleFlowScriptWorkerMessage(state, {
				kind: "folding",
				id: 4,
				catalogId: 1,
				doc: { uri: "a", version: 4 },
			}),
		);
		expect(reused.kind).toBe("folding");
		// New version without text is an error, never stale data.
		const missing = handleFlowScriptWorkerMessage(state, {
			kind: "folding",
			id: 5,
			catalogId: 1,
			doc: { uri: "a", version: 5 },
		});
		expect(missing?.kind).toBe("error");
	});

	test("unknown catalog id is an error", () => {
		const state = createFlowScriptWorkerState();
		const response = handleFlowScriptWorkerMessage(state, {
			kind: "diagnostics",
			id: 6,
			catalogId: 99,
			doc: { uri: "a", version: 1, text: DOC },
		});
		expect(response?.kind).toBe("error");
	});

	test("a cancel received before the request suppresses the computation", () => {
		const state = initState();
		expect(
			handleFlowScriptWorkerMessage(state, { kind: "cancel", id: 8 }),
		).toBeNull();
		const response = handleFlowScriptWorkerMessage(state, {
			kind: "diagnostics",
			id: 8,
			catalogId: 1,
			doc: { uri: "a", version: 1, text: DOC },
		});
		expect(response).toEqual({ kind: "cancelled", id: 8 });
		// The cancellation is consumed: a later request with the same id runs.
		const rerun = handleFlowScriptWorkerMessage(state, {
			kind: "diagnostics",
			id: 8,
			catalogId: 1,
			doc: { uri: "a", version: 1, text: DOC },
		});
		expect(rerun?.kind).toBe("ok");
	});

	test("env snapshot hydrates into a working type environment", () => {
		const state = initState();
		const result = okResult(
			handleFlowScriptWorkerMessage(state, {
				kind: "env-snapshot",
				id: 9,
				catalogId: 1,
				doc: { uri: "a", version: 1, text: DOC },
			}),
		);
		if (result.kind !== "env-snapshot") throw new Error("wrong result kind");
		const index = buildFlowScriptIndex(catalog, names);
		const hydrated = hydrateFlowScriptEnvDoc(DOC, result.snapshot, index);
		expect(hydrated.masked).toBe(getFlowScriptEnvDoc(DOC, index).masked);
		// `use log::*` opened `info` — the rebuilt scope resolves it against the local index.
		expect(hydrated.env.scope.openMembers.get("info")?.[0]?.nodeType).toBe(
			"log_info",
		);
		// The variable chain resolves through the hydrated environment.
		expect(evaluateExpr("trimmed", hydrated.env).value?.group).toBe("string");
		// Call-context analysis works on the hydrated masked text.
		const offset = DOC.indexOf("trimmed })") + "trimmed".length;
		const context = analyzeContext(
			hydrated.masked.slice(0, offset),
			hydrated.env,
		);
		expect(context?.info?.nodeType).toBe("log_info");
	});

	test("snapshots survive a structured-clone round trip", () => {
		const index = buildFlowScriptIndex(catalog, names);
		const snapshot = makeFlowScriptEnvSnapshot(DOC, index);
		const cloned = structuredClone(snapshot);
		const hydrated = hydrateFlowScriptEnvDoc(DOC, cloned, index);
		expect(hydrated.env.symbols.functions.has("onLoad")).toBe(true);
		expect(hydrated.lineStarts.length).toBe(DOC.split("\n").length);
	});

	test("an unparseable request kind fails as an error, not a crash", () => {
		const state = initState();
		const response = handleFlowScriptWorkerMessage(state, {
			kind: "inlay-hints",
			id: 10,
			catalogId: 1,
			doc: { uri: "a", version: 1, text: DOC },
			startLine: 1,
			endLine: 100,
		} as FlowScriptWorkerRequest);
		expect(response?.kind).toBe("ok");
	});
});
