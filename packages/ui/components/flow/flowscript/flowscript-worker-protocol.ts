/**
 * Typed protocol between the FlowScript worker client (main thread) and the
 * FlowScript language worker, plus the pure message handler both the worker
 * shim and the unit tests run.
 *
 * Design constraints:
 * - The catalog crosses the boundary once per identity (`init-catalog`), never
 *   per request; requests reference it by `catalogId`.
 * - Document text crosses once per (uri, version); later requests for the same
 *   version omit it.
 * - Every result is structured-cloneable plain data (Maps/Sets included) —
 *   never a `FlowScriptIndex` or `FlowScriptNodeInfo` graph.
 */

import type { FlowScriptNamesTable } from "../../../lib/flowscript/names";
import type { INode } from "../../../lib/schema/flow/node";
import {
	type DocumentSymbols,
	type FlowScriptIndex,
	type FlowScriptRawDiagnostic,
	type Span,
	type TypeEnv,
	type UseDeclaration,
	type ValueType,
	buildFlowScriptIndex,
	buildUseScopeFor,
	computeFlowScriptRawDiagnostics,
	getFlowScriptEnvDoc,
} from "./flowscript-language";
import {
	type FlowScriptDocumentSymbol,
	type FlowScriptEnvContext,
	type FlowScriptFoldingRange,
	type FlowScriptInlayHint,
	analyzeFlowScriptDocument,
	buildFlowScriptDocumentSymbols,
	buildFlowScriptFoldingRanges,
	buildFlowScriptInlayHints,
	buildFlowScriptSemanticTokens,
} from "./flowscript-language-features";

// ---------------------------------------------------------------------------
// Environment snapshot (worker → main): the serializable parts of an analyzed
// document's type environment. The `use`-scope is rebuilt against the local
// index on hydration because it holds references into the catalog graph.
// ---------------------------------------------------------------------------

export interface FlowScriptEnvSnapshot {
	masked: string;
	lineStarts: number[];
	templateExprs: Span[];
	templates: Span[];
	uses: UseDeclaration[];
	variables: Map<string, string | undefined>;
	functions: Set<string>;
	functionReceivers: Map<string, string | undefined>;
	interfaces: Set<string>;
	docVars: Map<string, ValueType>;
	exprs: Map<string, string>;
	loops: Map<string, string>;
	indexVars: Set<string>;
}

function computeLineStartOffsets(text: string): number[] {
	const starts = [0];
	for (let i = 0; i < text.length; i++) {
		if (text[i] === "\n") starts.push(i + 1);
	}
	return starts;
}

export function makeFlowScriptEnvSnapshot(
	text: string,
	index: FlowScriptIndex,
): FlowScriptEnvSnapshot {
	const envDoc = getFlowScriptEnvDoc(text, index);
	const { env } = envDoc;
	return {
		masked: envDoc.masked,
		lineStarts: computeLineStartOffsets(text),
		templateExprs: envDoc.templateExprs,
		templates: envDoc.templates,
		uses: env.symbols.uses,
		variables: env.symbols.variables,
		functions: env.symbols.functions,
		functionReceivers: env.symbols.functionReceivers,
		interfaces: env.symbols.interfaces,
		docVars: env.docVars,
		exprs: env.vars.exprs,
		loops: env.vars.loops,
		indexVars: env.vars.indexVars,
	};
}

/** Rebuilds a usable environment document from a snapshot against the local index. */
export function hydrateFlowScriptEnvDoc(
	text: string,
	snapshot: FlowScriptEnvSnapshot,
	index: FlowScriptIndex,
): FlowScriptEnvContext {
	const symbols: DocumentSymbols = {
		variables: snapshot.variables,
		functions: snapshot.functions,
		functionReceivers: snapshot.functionReceivers,
		interfaces: snapshot.interfaces,
		uses: snapshot.uses,
	};
	const env: TypeEnv = {
		index,
		scope: buildUseScopeFor(snapshot.uses, index),
		symbols,
		docVars: snapshot.docVars,
		vars: {
			exprs: snapshot.exprs,
			loops: snapshot.loops,
			indexVars: snapshot.indexVars,
		},
	};
	return {
		text,
		masked: snapshot.masked,
		lineStarts: snapshot.lineStarts,
		templateExprs: snapshot.templateExprs,
		templates: snapshot.templates,
		index,
		env,
		uses: snapshot.uses,
	};
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

export interface FlowScriptDocPayload {
	uri: string;
	version: number;
	/** Full text; omitted when this (uri, version) pair was already sent. */
	text?: string;
}

export type FlowScriptDocRequestKind =
	| "diagnostics"
	| "semantic-tokens"
	| "folding"
	| "document-symbols"
	| "inlay-hints"
	| "env-snapshot";

interface DocRequestBase {
	id: number;
	catalogId: number;
	doc: FlowScriptDocPayload;
}

export type FlowScriptWorkerDocRequest =
	| ({ kind: "diagnostics" } & DocRequestBase)
	| ({ kind: "semantic-tokens" } & DocRequestBase)
	| ({ kind: "folding" } & DocRequestBase)
	| ({ kind: "document-symbols" } & DocRequestBase)
	| ({
			kind: "inlay-hints";
			startLine: number;
			endLine: number;
	  } & DocRequestBase)
	| ({ kind: "env-snapshot" } & DocRequestBase);

export type FlowScriptWorkerRequest =
	| {
			kind: "init-catalog";
			catalogId: number;
			nodes: INode[];
			names?: FlowScriptNamesTable;
	  }
	| { kind: "cancel"; id: number }
	| FlowScriptWorkerDocRequest;

export type FlowScriptWorkerResult =
	| { kind: "diagnostics"; markers: FlowScriptRawDiagnostic[] }
	| { kind: "semantic-tokens"; data: Uint32Array }
	| { kind: "folding"; ranges: FlowScriptFoldingRange[] }
	| { kind: "document-symbols"; symbols: FlowScriptDocumentSymbol[] }
	| { kind: "inlay-hints"; hints: FlowScriptInlayHint[] }
	| { kind: "env-snapshot"; snapshot: FlowScriptEnvSnapshot };

export type FlowScriptWorkerResponse =
	| { kind: "ok"; id: number; result: FlowScriptWorkerResult }
	| { kind: "cancelled"; id: number }
	| { kind: "error"; id: number; message: string };

// ---------------------------------------------------------------------------
// Worker state + handler (pure; runs in the worker shim and in unit tests)
// ---------------------------------------------------------------------------

const MAX_CATALOGS = 2;
const MAX_CANCELLED = 512;

export interface FlowScriptWorkerState {
	catalogs: Map<number, FlowScriptIndex>;
	docs: Map<string, { version: number; text: string }>;
	cancelled: Set<number>;
}

export function createFlowScriptWorkerState(): FlowScriptWorkerState {
	return { catalogs: new Map(), docs: new Map(), cancelled: new Set() };
}

function resolveDocText(
	state: FlowScriptWorkerState,
	doc: FlowScriptDocPayload,
): string | undefined {
	if (typeof doc.text === "string") {
		state.docs.set(doc.uri, { version: doc.version, text: doc.text });
		return doc.text;
	}
	const cached = state.docs.get(doc.uri);
	if (cached && cached.version === doc.version) return cached.text;
	return undefined;
}

export function handleFlowScriptWorkerMessage(
	state: FlowScriptWorkerState,
	request: FlowScriptWorkerRequest,
): FlowScriptWorkerResponse | null {
	if (request.kind === "init-catalog") {
		state.catalogs.set(
			request.catalogId,
			buildFlowScriptIndex(request.nodes, request.names),
		);
		while (state.catalogs.size > MAX_CATALOGS) {
			const oldest = state.catalogs.keys().next().value;
			if (oldest === undefined) break;
			state.catalogs.delete(oldest);
		}
		return null;
	}
	if (request.kind === "cancel") {
		if (state.cancelled.size >= MAX_CANCELLED) state.cancelled.clear();
		state.cancelled.add(request.id);
		return null;
	}
	if (state.cancelled.delete(request.id)) {
		return { kind: "cancelled", id: request.id };
	}
	const index = state.catalogs.get(request.catalogId);
	if (!index) {
		return {
			kind: "error",
			id: request.id,
			message: `Unknown FlowScript catalog id ${request.catalogId}; send init-catalog first.`,
		};
	}
	const text = resolveDocText(state, request.doc);
	if (text === undefined) {
		return {
			kind: "error",
			id: request.id,
			message: `Missing document text for '${request.doc.uri}' v${request.doc.version}.`,
		};
	}
	try {
		switch (request.kind) {
			case "diagnostics":
				return {
					kind: "ok",
					id: request.id,
					result: {
						kind: "diagnostics",
						markers: computeFlowScriptRawDiagnostics(text, index),
					},
				};
			case "semantic-tokens":
				return {
					kind: "ok",
					id: request.id,
					result: {
						kind: "semantic-tokens",
						data: buildFlowScriptSemanticTokens(
							analyzeFlowScriptDocument(text, index),
						),
					},
				};
			case "folding":
				return {
					kind: "ok",
					id: request.id,
					result: {
						kind: "folding",
						ranges: buildFlowScriptFoldingRanges(
							analyzeFlowScriptDocument(text, index),
						),
					},
				};
			case "document-symbols":
				return {
					kind: "ok",
					id: request.id,
					result: {
						kind: "document-symbols",
						symbols: buildFlowScriptDocumentSymbols(
							analyzeFlowScriptDocument(text, index),
						),
					},
				};
			case "inlay-hints":
				return {
					kind: "ok",
					id: request.id,
					result: {
						kind: "inlay-hints",
						hints: buildFlowScriptInlayHints(
							analyzeFlowScriptDocument(text, index),
							request.startLine,
							request.endLine,
						),
					},
				};
			case "env-snapshot":
				return {
					kind: "ok",
					id: request.id,
					result: {
						kind: "env-snapshot",
						snapshot: makeFlowScriptEnvSnapshot(text, index),
					},
				};
		}
	} catch (error) {
		return {
			kind: "error",
			id: request.id,
			message: `FlowScript worker request '${request.kind}' failed: ${
				error instanceof Error ? error.message : String(error)
			}`,
		};
	}
	return {
		kind: "error",
		id: (request as FlowScriptWorkerDocRequest).id,
		message: "Unhandled FlowScript worker request kind.",
	};
}
