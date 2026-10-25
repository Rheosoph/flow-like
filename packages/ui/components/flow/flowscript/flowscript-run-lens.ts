/**
 * Rust-test-style run lenses for the FlowScript editor: every top-level EVENT
 * block gets a "▶ Run" CodeLens on its header line (plus "☁" when the board can
 * execute remotely), resolved to the event's entry node through the `//@n:`
 * anchor the renderer leaves on that line. Functions are not entry points and
 * never get a lens.
 *
 * Gating mirrors the board, not the buffer alone:
 * - read-only / version-pinned panels render NO lens at all — a pinned view's
 *   entry ids may no longer exist on the live board, and an inert lens on every
 *   event would be permanent noise on a surface that can never run;
 * - a dirty buffer renders one unclickable "apply before running" lens per
 *   event instead of hiding the affordance — running the stale board while the
 *   text shows something else would confuse, but a vanishing button would too.
 */

import type { Monaco } from "@monaco-editor/react";
import type { INode } from "../../../lib/schema/flow/node";
import {
	type FlowScriptAnchorIndex,
	anchorAtLine,
	parseFlowScriptAnchors,
} from "./flowscript-anchors";
import {
	FLOWSCRIPT_LANGUAGE_ID,
	getFlowScriptIndex,
} from "./flowscript-language";
import {
	type FlowScriptDeclaration,
	analyzeFlowScriptDocument,
} from "./flowscript-language-features";
import type { FlowScriptWorkerRequests } from "./flowscript-worker-contract";

export type FlowScriptRunMode = "local" | "remote";

export interface FlowScriptRunCapability {
	local: boolean;
	remote: boolean;
}

export interface FlowScriptRunLensGate {
	readOnly: boolean;
	dirty: boolean;
	/**
	 * Entry-node id → what the board allows. Absent (older hosts) degrades to
	 * local-only lenses; an id missing from the map gets no lens — its event no
	 * longer exists on the board.
	 */
	runnableNodes?: ReadonlyMap<string, FlowScriptRunCapability>;
}

export type FlowScriptRunLensKind = "run-local" | "run-remote" | "apply-first";

export interface FlowScriptRunLens {
	/** 1-based event header line the lens renders above. */
	line: number;
	nodeId: string;
	eventName: string;
	kind: FlowScriptRunLensKind;
}

export interface FlowScriptRunLensDoc {
	declarations: readonly FlowScriptDeclaration[];
	/** Offset of each line start, index 0 = line 1 (see computeLineStarts). */
	lineStarts: readonly number[];
}

function lineOfOffset(lineStarts: readonly number[], offset: number): number {
	let lo = 0;
	let hi = lineStarts.length - 1;
	while (lo < hi) {
		const mid = (lo + hi + 1) >> 1;
		if (lineStarts[mid] <= offset) lo = mid;
		else hi = mid - 1;
	}
	return lo + 1;
}

/**
 * Pure lens derivation: top-level `event` declarations whose header line holds
 * a node anchor, expanded per the gate. Exported for tests; the provider below
 * is a thin Monaco adapter around it.
 */
export function deriveFlowScriptRunLenses(
	doc: FlowScriptRunLensDoc,
	anchorIndex: FlowScriptAnchorIndex,
	gate: FlowScriptRunLensGate,
): FlowScriptRunLens[] {
	return deriveEventLenses(
		doc.declarations
			.filter((declaration) => declaration.kind === "event")
			.map((declaration) => ({
				line: lineOfOffset(doc.lineStarts, declaration.span.start),
				name: declaration.name,
			})),
		anchorIndex,
		gate,
	);
}

function deriveEventLenses(
	events: readonly { line: number; name: string }[],
	anchorIndex: FlowScriptAnchorIndex,
	gate: FlowScriptRunLensGate,
): FlowScriptRunLens[] {
	if (gate.readOnly) return [];
	const lenses: FlowScriptRunLens[] = [];
	for (const { line, name } of events) {
		const anchor = anchorAtLine(anchorIndex, line);
		if (!anchor || anchor.kind !== "node") continue;
		const base = {
			line,
			nodeId: anchor.id,
			eventName: name,
		};
		if (gate.dirty) {
			lenses.push({ ...base, kind: "apply-first" });
			continue;
		}
		const capability = gate.runnableNodes?.get(anchor.id) ?? {
			local: !gate.runnableNodes,
			remote: false,
		};
		if (capability.local) lenses.push({ ...base, kind: "run-local" });
		if (capability.remote) lenses.push({ ...base, kind: "run-remote" });
	}
	return lenses;
}

export interface FlowScriptRunLensLabels {
	runEvent: string;
	runRemote: string;
	applyBeforeRun: string;
}

interface RunLensCommandArg {
	flowscriptRunLens: true;
	nodeId: string;
	mode: FlowScriptRunMode;
}

function isRunLensCommandArg(value: unknown): value is RunLensCommandArg {
	return (
		typeof value === "object" &&
		value !== null &&
		(value as RunLensCommandArg).flowscriptRunLens === true &&
		typeof (value as RunLensCommandArg).nodeId === "string"
	);
}

interface RunLensEditorLike {
	/** Registers an editor-scoped command; the returned id backs the lens command. */
	addCommand(
		keybinding: number,
		handler: (...args: unknown[]) => void,
	): string | null;
	getModel(): unknown;
}

export interface RegisterFlowScriptRunLensOptions {
	editor: RunLensEditorLike;
	getCatalogNodes: () => INode[] | undefined;
	workerRequests: Pick<FlowScriptWorkerRequests, "requestDocumentSymbols">;
	getGate: () => FlowScriptRunLensGate;
	getLabels: () => FlowScriptRunLensLabels;
	onRun: (nodeId: string, mode: FlowScriptRunMode) => void;
}

export interface FlowScriptRunLensHandle {
	/** Re-request lenses after gate/label changes Monaco cannot observe itself. */
	refresh: () => void;
	dispose: () => void;
}

/**
 * Registers the CodeLens provider for this panel's editor. Scoped to the
 * editor's own model so a second mounted panel can never double its lenses.
 */
export function registerFlowScriptRunLens(
	monaco: Monaco,
	options: RegisterFlowScriptRunLensOptions,
): FlowScriptRunLensHandle {
	const emitter = new monaco.Emitter<void>();
	const commandId = options.editor.addCommand(0, (...args: unknown[]) => {
		// Monaco may prepend a services accessor; find our marked argument.
		const payload = args.find(isRunLensCommandArg);
		if (payload) options.onRun(payload.nodeId, payload.mode);
	});
	const provider = monaco.languages.registerCodeLensProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			// biome-ignore lint/suspicious/noExplicitAny: Emitter<void> vs IEvent<provider> — Monaco only uses it as a change signal
			onDidChange: emitter.event as any,
			provideCodeLenses: (model, token) => {
				if (
					(model as unknown) !== options.editor.getModel() ||
					options.getGate().readOnly ||
					token.isCancellationRequested
				) {
					return { lenses: [], dispose: () => {} };
				}
				const text = model.getValue();
				const version = model.getVersionId();
				const catalog = options.getCatalogNodes();
				const toResult = (derived: FlowScriptRunLens[]) => {
					const labels = options.getLabels();
					const lenses = derived.map((lens) => ({
						range: new monaco.Range(lens.line, 1, lens.line, 1),
						command:
							lens.kind === "apply-first"
								? { id: "", title: labels.applyBeforeRun }
								: {
										id: commandId ?? "",
										title:
											lens.kind === "run-local"
												? `▶ ${labels.runEvent}`
												: `☁ ${labels.runRemote}`,
										tooltip: lens.eventName,
										arguments: [
											{
												flowscriptRunLens: true,
												nodeId: lens.nodeId,
												mode: lens.kind === "run-local" ? "local" : "remote",
											} satisfies RunLensCommandArg,
										],
									},
					}));
					return { lenses, dispose: () => {} };
				};
				const inThread = () =>
					toResult(
						deriveFlowScriptRunLenses(
							analyzeFlowScriptDocument(text, getFlowScriptIndex(catalog)),
							parseFlowScriptAnchors(text),
							options.getGate(),
						),
					);
				// Reuse the worker's outline analysis. CodeLens runs after edits too, so
				// analyzing here would put large-document work back on the UI thread.
				const request = options.workerRequests.requestDocumentSymbols(
					model,
					catalog,
					token,
				);
				if (!request) return inThread();
				return request.then((outcome) => {
					if (
						outcome.status === "cancelled" ||
						token.isCancellationRequested ||
						model !== options.editor.getModel() ||
						model.getVersionId() !== version
					)
						return toResult([]);
					if (outcome.status === "failed") return inThread();
					return toResult(
						deriveEventLenses(
							outcome.value
								.filter((symbol) => symbol.declarationKind === "event")
								.map((symbol) => ({
									line: symbol.range.startLineNumber,
									name: symbol.name,
								})),
							parseFlowScriptAnchors(text),
							options.getGate(),
						),
					);
				});
			},
		},
	);
	return {
		refresh: () => emitter.fire(undefined),
		dispose: () => {
			provider.dispose();
			emitter.dispose();
		},
	};
}
