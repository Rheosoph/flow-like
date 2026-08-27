/**
 * Main-thread client for the FlowScript language worker.
 *
 * Decides per request whether the worker is worth using (available, document
 * large enough, model versioned), ships the catalog once per identity and the
 * document text once per version, coalesces duplicate requests for the same
 * version, and cancels superseded ones (latest text wins). Every caller keeps
 * a synchronous in-thread fallback for SSR, tests and small documents.
 */

import type { Monaco } from "@monaco-editor/react";
import { getFlowScriptNamesTable } from "../../../lib/flowscript/names";
import type { INode } from "../../../lib/schema/flow/node";
import {
	type FlowScriptBoardScope,
	type FlowScriptRawDiagnostic,
	computeFlowScriptDiagnostics,
	flowScriptMarkersFromRaw,
	getFlowScriptIndex,
} from "./flowscript-language";
import type {
	FlowScriptDocumentSymbol,
	FlowScriptEnvContext,
	FlowScriptFoldingRange,
	FlowScriptInlayHint,
} from "./flowscript-language-features";
import type {
	CancellationTokenLike,
	FlowScriptWorkerModelLike,
	FlowScriptWorkerOutcome,
	FlowScriptWorkerRequests,
} from "./flowscript-worker-contract";
import {
	type FlowScriptDocRequestKind,
	type FlowScriptEnvSnapshot,
	type FlowScriptWorkerResponse,
	type FlowScriptWorkerResult,
	hydrateFlowScriptEnvDoc,
} from "./flowscript-worker-protocol";

export type {
	CancellationTokenLike,
	FlowScriptWorkerModelLike,
	FlowScriptWorkerOutcome,
} from "./flowscript-worker-contract";

/**
 * Below this text length the postMessage round trip costs about as much as
 * computing in place (measured: ~8 KB of FlowScript analyzes in ~1.5 ms).
 */
export const FLOWSCRIPT_WORKER_MIN_TEXT_LENGTH = 8_000;

interface PendingEntry {
	key: string;
	kind: FlowScriptDocRequestKind;
	resolve: (outcome: FlowScriptWorkerOutcome<FlowScriptWorkerResult>) => void;
}

interface InflightEntry {
	id: number;
	version: number;
	extraKey: string;
	promise: Promise<FlowScriptWorkerOutcome<FlowScriptWorkerResult>>;
}

let worker: Worker | null | undefined;
let requestSeq = 0;
let catalogSeq = 0;
let lastCatalog:
	| { nodes: INode[] | undefined; names: unknown; id: number }
	| undefined;
const pending = new Map<number, PendingEntry>();
const inflight = new Map<string, InflightEntry>();
const sentDocs = new Map<string, number>();

function resetWorkerState(reason: string): void {
	for (const entry of pending.values()) entry.resolve({ status: "failed" });
	pending.clear();
	inflight.clear();
	sentDocs.clear();
	lastCatalog = undefined;
	if (reason) {
		console.warn(
			`[flowscript] language worker unavailable (${reason}); falling back to in-thread analysis.`,
		);
	}
}

function getWorker(): Worker | null {
	if (worker !== undefined) return worker;
	if (typeof window === "undefined" || typeof Worker === "undefined") {
		worker = null;
		return worker;
	}
	try {
		worker = new Worker(
			new URL("./flowscript-language.worker.ts", import.meta.url),
			{ type: "module" },
		);
		worker.onmessage = (event: MessageEvent<FlowScriptWorkerResponse>) => {
			const response = event.data;
			const entry = pending.get(response.id);
			if (!entry) return;
			pending.delete(response.id);
			const current = inflight.get(entry.key);
			if (current?.id === response.id) inflight.delete(entry.key);
			if (response.kind === "ok") {
				entry.resolve({ status: "ok", value: response.result });
			} else if (response.kind === "cancelled") {
				entry.resolve({ status: "cancelled" });
			} else {
				console.warn(`[flowscript] worker request failed: ${response.message}`);
				entry.resolve({ status: "failed" });
			}
		};
		worker.onerror = () => {
			worker?.terminate();
			worker = null;
			resetWorkerState("worker crashed");
		};
	} catch (error) {
		worker = null;
		resetWorkerState(
			error instanceof Error ? error.message : "failed to start",
		);
	}
	return worker;
}

function ensureCatalog(w: Worker, nodes: INode[] | undefined): number {
	const names = getFlowScriptNamesTable();
	if (
		lastCatalog &&
		lastCatalog.nodes === nodes &&
		lastCatalog.names === names
	) {
		return lastCatalog.id;
	}
	const id = ++catalogSeq;
	lastCatalog = { nodes, names, id };
	w.postMessage({
		kind: "init-catalog",
		catalogId: id,
		nodes: nodes ?? [],
		names,
	});
	return id;
}

function cancelRequestId(id: number): void {
	const entry = pending.get(id);
	if (!entry) return;
	pending.delete(id);
	const current = inflight.get(entry.key);
	if (current?.id === id) inflight.delete(entry.key);
	entry.resolve({ status: "cancelled" });
	try {
		worker?.postMessage({ kind: "cancel", id });
	} catch {
		// The worker is gone; state was reset by onerror.
	}
}

/**
 * Requests one language product from the worker, or returns `null` when the
 * caller should compute synchronously in-thread (no Worker/DOM, unversioned
 * model, document below the size threshold, or worker start-up failed).
 */
function requestFromWorker(
	kind: FlowScriptDocRequestKind,
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	extra: Record<string, unknown> | undefined,
	token: CancellationTokenLike | undefined,
): Promise<FlowScriptWorkerOutcome<FlowScriptWorkerResult>> | null {
	const version = model.getVersionId?.();
	if (version === undefined) return null;
	const text = model.getValue();
	if (text.length < FLOWSCRIPT_WORKER_MIN_TEXT_LENGTH) return null;
	const w = getWorker();
	if (!w) return null;

	const uriKey = String(model.uri);
	const key = `${kind}:${uriKey}`;
	const extraKey = extra ? JSON.stringify(extra) : "";
	const existing = inflight.get(key);
	if (
		existing &&
		existing.version === version &&
		existing.extraKey === extraKey
	) {
		return existing.promise;
	}
	if (existing) cancelRequestId(existing.id);

	const catalogId = ensureCatalog(w, nodes);
	const id = ++requestSeq;
	const doc = {
		uri: uriKey,
		version,
		text: sentDocs.get(uriKey) === version ? undefined : text,
	};
	sentDocs.set(uriKey, version);
	const promise = new Promise<FlowScriptWorkerOutcome<FlowScriptWorkerResult>>(
		(resolve) => {
			pending.set(id, { key, kind, resolve });
		},
	);
	inflight.set(key, { id, version, extraKey, promise });
	if (token?.isCancellationRequested) {
		cancelRequestId(id);
		return promise;
	}
	token?.onCancellationRequested?.(() => cancelRequestId(id));
	try {
		w.postMessage({ kind, id, catalogId, doc, ...extra });
	} catch (error) {
		cancelRequestId(id);
		worker?.terminate();
		worker = null;
		resetWorkerState(
			error instanceof Error ? error.message : "postMessage failed",
		);
		return null;
	}
	return promise;
}

function mapOutcome<K extends FlowScriptWorkerResult["kind"], T>(
	request: Promise<FlowScriptWorkerOutcome<FlowScriptWorkerResult>>,
	kind: K,
	map: (result: Extract<FlowScriptWorkerResult, { kind: K }>) => T,
): Promise<FlowScriptWorkerOutcome<T>> {
	return request.then((outcome) => {
		if (outcome.status !== "ok") return outcome;
		if (outcome.value.kind !== kind) return { status: "failed" as const };
		return {
			status: "ok" as const,
			value: map(outcome.value as Extract<FlowScriptWorkerResult, { kind: K }>),
		};
	});
}

export function requestFlowScriptWorkerDiagnostics(
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	board?: FlowScriptBoardScope,
	token?: CancellationTokenLike,
): Promise<FlowScriptWorkerOutcome<FlowScriptRawDiagnostic[]>> | null {
	// Passed as `extra` so a board change (module created/renamed) also invalidates the
	// coalescing key — a stale request for the same text must not answer the new question.
	const request = requestFromWorker(
		"diagnostics",
		model,
		nodes,
		board ? { board } : undefined,
		token,
	);
	return request && mapOutcome(request, "diagnostics", (r) => r.markers);
}

export function requestFlowScriptWorkerSemanticTokens(
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	token?: CancellationTokenLike,
): Promise<FlowScriptWorkerOutcome<Uint32Array>> | null {
	const request = requestFromWorker(
		"semantic-tokens",
		model,
		nodes,
		undefined,
		token,
	);
	return request && mapOutcome(request, "semantic-tokens", (r) => r.data);
}

export function requestFlowScriptWorkerFolding(
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	token?: CancellationTokenLike,
): Promise<FlowScriptWorkerOutcome<FlowScriptFoldingRange[]>> | null {
	const request = requestFromWorker("folding", model, nodes, undefined, token);
	return request && mapOutcome(request, "folding", (r) => r.ranges);
}

export function requestFlowScriptWorkerDocumentSymbols(
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	token?: CancellationTokenLike,
): Promise<FlowScriptWorkerOutcome<FlowScriptDocumentSymbol[]>> | null {
	const request = requestFromWorker(
		"document-symbols",
		model,
		nodes,
		undefined,
		token,
	);
	return request && mapOutcome(request, "document-symbols", (r) => r.symbols);
}

export function requestFlowScriptWorkerInlayHints(
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	startLine: number,
	endLine: number,
	token?: CancellationTokenLike,
): Promise<FlowScriptWorkerOutcome<FlowScriptInlayHint[]>> | null {
	const request = requestFromWorker(
		"inlay-hints",
		model,
		nodes,
		{ startLine, endLine },
		token,
	);
	return request && mapOutcome(request, "inlay-hints", (r) => r.hints);
}

export function requestFlowScriptWorkerEnvDoc(
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	token?: CancellationTokenLike,
): Promise<FlowScriptWorkerOutcome<FlowScriptEnvContext>> | null {
	const request = requestFromWorker(
		"env-snapshot",
		model,
		nodes,
		undefined,
		token,
	);
	if (!request) return null;
	const text = model.getValue();
	return mapOutcome(
		request,
		"env-snapshot",
		(r: { snapshot: FlowScriptEnvSnapshot }) =>
			hydrateFlowScriptEnvDoc(text, r.snapshot, getFlowScriptIndex(nodes)),
	);
}

/** The worker implementation injected into the main-thread Monaco provider facade. */
export const flowScriptWorkerRequests = {
	requestEnvDoc: requestFlowScriptWorkerEnvDoc,
	requestDocumentSymbols: requestFlowScriptWorkerDocumentSymbols,
	requestFolding: requestFlowScriptWorkerFolding,
	requestInlayHints: requestFlowScriptWorkerInlayHints,
	requestSemanticTokens: requestFlowScriptWorkerSemanticTokens,
} satisfies FlowScriptWorkerRequests;

/**
 * Client-lint markers for the panel: computed in the worker for large
 * documents, in-thread otherwise. Always resolves to Monaco-ready markers.
 */
export function computeFlowScriptMarkersPreferWorker(
	monaco: Monaco,
	model: FlowScriptWorkerModelLike,
	nodes: INode[] | undefined,
	board?: FlowScriptBoardScope,
): unknown[] | Promise<unknown[]> {
	const request = requestFlowScriptWorkerDiagnostics(model, nodes, board);
	if (!request)
		return computeFlowScriptDiagnostics(monaco, model.getValue(), nodes, board)
			.markers;
	return request.then((outcome) => {
		if (outcome.status === "ok")
			return flowScriptMarkersFromRaw(monaco, outcome.value);
		if (outcome.status === "cancelled") return [];
		return computeFlowScriptDiagnostics(monaco, model.getValue(), nodes, board)
			.markers;
	});
}
