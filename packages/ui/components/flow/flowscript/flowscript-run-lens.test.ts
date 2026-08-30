import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import { buildFlowScriptIndex } from "./flowscript-language";
import { analyzeFlowScriptDocument } from "./flowscript-language-features";
import {
	type FlowScriptRunLensGate,
	deriveFlowScriptRunLenses,
} from "./flowscript-run-lens";

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
