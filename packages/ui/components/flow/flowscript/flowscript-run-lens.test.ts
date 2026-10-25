import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Monaco } from "@monaco-editor/react";
import type { INode } from "../../../lib/schema/flow/node";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import { buildFlowScriptIndex } from "./flowscript-language";
import {
	type FlowScriptDocumentSymbol,
	analyzeFlowScriptDocument,
	buildFlowScriptDocumentSymbols,
} from "./flowscript-language-features";
import {
	type FlowScriptRunLensGate,
	deriveFlowScriptRunLenses,
	registerFlowScriptRunLens,
} from "./flowscript-run-lens";
import type {
	FlowScriptWorkerOutcome,
	FlowScriptWorkerRequests,
} from "./flowscript-worker-contract";

const FIXTURE_DIR = join(import.meta.dir, "../../../../../tests/ast");

function fixture(name: string): string {
	return readFileSync(join(FIXTURE_DIR, name), "utf8");
}

const EMPTY_INDEX = buildFlowScriptIndex([], {});

function lensesFor(text: string, gate: FlowScriptRunLensGate) {
	return deriveFlowScriptRunLenses(
		analyzeFlowScriptDocument(text, EMPTY_INDEX),
		parseFlowScriptAnchors(text),
		gate,
	);
}

const DOC = [
	"use log::*",
	"",
	"function helper(x: string): (out: string) {   //@l:layerhelper000000000001",
	"    const y = x   //@n:nodeinsidehelper0000001",
	"}",
	"",
	"eventsSimple onLoad() {   //@n:entryonload000000000001",
	'    info({ message: "hi" })   //@n:nodeinsideonload0000001',
	"}",
	"",
	"eventsChat onChat(history: History) {   //@n:entryonchat000000000001",
	'    info({ message: "chat" })   //@n:nodeinsideonchat0000001',
	"}",
	"",
	"eventsSimple noAnchor() {",
	'    info({ message: "orphan" })',
	"}",
].join("\n");

const CLEAN: FlowScriptRunLensGate = { readOnly: false, dirty: false };

describe("FlowScript run lens derivation", () => {
	test("events get a local lens on their header line; functions and un-anchored headers get none", () => {
		const lenses = lensesFor(DOC, CLEAN);
		expect(lenses).toEqual([
			{
				line: 7,
				nodeId: "entryonload000000000001",
				eventName: "onLoad",
				kind: "run-local",
			},
			{
				line: 11,
				nodeId: "entryonchat000000000001",
				eventName: "onChat",
				kind: "run-local",
			},
		]);
	});

	test("capability map gates modes and drops events missing from the board", () => {
		const lenses = lensesFor(DOC, {
			...CLEAN,
			runnableNodes: new Map([
				["entryonload000000000001", { local: true, remote: true }],
				// onChat's entry no longer exists on the board — no lens for it.
			]),
		});
		expect(lenses).toEqual([
			{
				line: 7,
				nodeId: "entryonload000000000001",
				eventName: "onLoad",
				kind: "run-local",
			},
			{
				line: 7,
				nodeId: "entryonload000000000001",
				eventName: "onLoad",
				kind: "run-remote",
			},
		]);
	});

	test("remote-only execution mode yields only the remote lens", () => {
		const lenses = lensesFor(DOC, {
			...CLEAN,
			runnableNodes: new Map([
				["entryonload000000000001", { local: false, remote: true }],
				["entryonchat000000000001", { local: false, remote: false }],
			]),
		});
		expect(lenses).toEqual([
			{
				line: 7,
				nodeId: "entryonload000000000001",
				eventName: "onLoad",
				kind: "run-remote",
			},
		]);
	});

	test("a dirty buffer replaces every run lens with one unclickable apply-first lens", () => {
		const lenses = lensesFor(DOC, {
			readOnly: false,
			dirty: true,
			runnableNodes: new Map([
				["entryonload000000000001", { local: true, remote: true }],
			]),
		});
		expect(lenses.map((lens) => lens.kind)).toEqual([
			"apply-first",
			"apply-first",
		]);
		expect(lenses.map((lens) => lens.line)).toEqual([7, 11]);
	});

	test("read-only (version-pinned) panels render no lens at all", () => {
		expect(lensesFor(DOC, { readOnly: true, dirty: false })).toEqual([]);
	});

	test("resolves every event of a real rendered board through its header anchor", () => {
		const text = fixture("ttwctnp08u18sg2z6nmcqqak.anchored.flow");
		const anchors = parseFlowScriptAnchors(text);
		const lenses = lensesFor(text, CLEAN);
		expect(lenses.length).toBeGreaterThan(20);
		const chatEvent = lenses.find((lens) => lens.eventName === "chatEvent");
		expect(chatEvent).toMatchObject({
			nodeId: "liqnumu9en44cq30tu9t5kez",
			kind: "run-local",
		});
		// The function layer header (`function constructPrompt … //@l:`) is not an entry point.
		expect(lenses.some((lens) => lens.eventName === "constructPrompt")).toBe(
			false,
		);
		for (const lens of lenses) {
			// Every lens sits on a line whose anchor is the node id it runs.
			expect(anchors.byLine.get(lens.line)?.id).toBe(lens.nodeId);
			expect(anchors.byLine.get(lens.line)?.kind).toBe("node");
		}
	});
});

type CodeLensProvider = Parameters<
	Monaco["languages"]["registerCodeLensProvider"]
>[1];

function providerHarness(
	text: string,
	requestDocumentSymbols: FlowScriptWorkerRequests["requestDocumentSymbols"],
	catalog: INode[] = [],
) {
	let provider: CodeLensProvider | undefined;
	let version = 1;
	let gate = CLEAN;
	const model = {
		uri: "inmemory://run-lens",
		getValue: () => text,
		getVersionId: () => version,
	} as unknown as Parameters<CodeLensProvider["provideCodeLenses"]>[0];
	let activeModel: unknown = model;
	const token = {
		isCancellationRequested: false,
		onCancellationRequested: () => ({ dispose() {} }),
	};
	const monaco = {
		Emitter: class {
			event = () => ({ dispose() {} });
			fire() {}
			dispose() {}
		},
		Range: class {
			constructor(
				public startLineNumber: number,
				public startColumn: number,
				public endLineNumber: number,
				public endColumn: number,
			) {}
		},
		languages: {
			registerCodeLensProvider: (
				_language: string,
				value: CodeLensProvider,
			) => {
				provider = value;
				return { dispose() {} };
			},
		},
	} as unknown as Monaco;
	registerFlowScriptRunLens(monaco, {
		editor: { addCommand: () => "run", getModel: () => activeModel },
		getCatalogNodes: () => catalog,
		workerRequests: { requestDocumentSymbols },
		getGate: () => gate,
		getLabels: () => ({
			runEvent: "Run",
			runRemote: "Run on server",
			applyBeforeRun: "Apply first",
		}),
		onRun() {},
	});
	return {
		provide: () => {
			if (!provider) throw new Error("CodeLens provider was not registered");
			return provider.provideCodeLenses(model, token);
		},
		setGate: (next: FlowScriptRunLensGate) => {
			gate = next;
		},
		edit: () => version++,
		switchModel: () => {
			activeModel = undefined;
		},
		cancel: () => {
			token.isCancellationRequested = true;
		},
	};
}

function symbolsFor(text: string) {
	return buildFlowScriptDocumentSymbols(
		analyzeFlowScriptDocument(text, EMPTY_INDEX),
	);
}

describe("FlowScript run lens worker provider", () => {
	test("uses worker symbols without analyzing the catalog on the UI thread", async () => {
		const text = fixture("ttwctnp08u18sg2z6nmcqqak.anchored.flow");
		const symbols = symbolsFor(text);
		const catalog = [
			new Proxy({} as INode, {
				get() {
					throw new Error("catalog analyzed on the UI thread");
				},
			}),
		];
		let requests = 0;
		const harness = providerHarness(
			text,
			async () => {
				requests++;
				return { status: "ok", value: symbols };
			},
			catalog,
		);
		const result = await harness.provide();
		expect(requests).toBe(1);
		expect(
			result?.lenses.map((lens) => ({
				line: lens.range.startLineNumber,
				nodeId: lens.command?.arguments?.[0].nodeId,
				eventName: lens.command?.tooltip,
				kind: "run-local",
			})),
		).toEqual(lensesFor(text, CLEAN));
	});

	test("retains lenses when no worker is available", async () => {
		const result = await providerHarness(DOC, () => null).provide();
		expect(result?.lenses.map((lens) => lens.range.startLineNumber)).toEqual([
			7, 11,
		]);
	});

	test("does not offer run commands for handlers inside detached blocks", async () => {
		const text = [
			DOC,
			"detached {",
			"    onError() {   //@n:handlerindetached",
			"    }",
			"}",
		].join("\n");
		const result = await providerHarness(text, async () => ({
			status: "ok",
			value: symbolsFor(text),
		})).provide();
		expect(result?.lenses.map((lens) => lens.range.startLineNumber)).toEqual([
			7, 11,
		]);
	});

	test("does not request analysis for a read-only model", async () => {
		const harness = providerHarness(DOC, () => {
			throw new Error("read-only panel requested analysis");
		});
		harness.setGate({ readOnly: true, dirty: false });
		expect((await harness.provide())?.lenses).toEqual([]);
	});

	for (const change of ["edit", "switchModel", "cancel"] as const) {
		test(`discards worker symbols after ${change}`, async () => {
			let complete:
				| ((
						result: FlowScriptWorkerOutcome<FlowScriptDocumentSymbol[]>,
				  ) => void)
				| undefined;
			const harness = providerHarness(
				DOC,
				() =>
					new Promise((resolve) => {
						complete = resolve;
					}),
			);
			const result = harness.provide();
			harness[change]();
			complete?.({ status: "ok", value: symbolsFor(DOC) });
			expect((await result)?.lenses).toEqual([]);
		});
	}

	test("reads the current run gate when a worker request finishes", async () => {
		const harness = providerHarness(DOC, async () => ({
			status: "ok",
			value: symbolsFor(DOC),
		}));
		const result = harness.provide();
		harness.setGate({ readOnly: false, dirty: true });
		expect((await result)?.lenses.map((lens) => lens.command?.title)).toEqual([
			"Apply first",
			"Apply first",
		]);
	});
});
