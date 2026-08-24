import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import {
	deriveFlowScriptConflictLenses,
	intersectRemoteTouched,
	mergeFlowScript,
	resolveFlowScriptConflict,
	segmentFlowScriptUnits,
} from "./flowscript-merge";

const FIXTURE_DIR = join(import.meta.dir, "../../../../../tests/ast");

function fixture(name: string): string {
	return readFileSync(join(FIXTURE_DIR, name), "utf8");
}

const VAR_ID = "varalpha000000000001";
const MAIN_ID = "nodemain000000000001";
const X_ID = "nodex000000000000001";
const Y_ID = "nodey000000000000001";
const OTHER_ID = "nodeother00000000001";
const LOG_ID = "nodelog0000000000001";
const NEW_ID = "nodenew0000000000001";

const BASE_LINES = [
	"use std::*",
	"",
	"interface Foo {",
	"    a: string;",
	"}",
	`const alpha = 1   //@v:${VAR_ID}`,
	"",
	`eventsGeneric main(payload: Struct) {   //@n:${MAIN_ID}`,
	`    const x = now()   //@n:${X_ID}`,
	`    const y = add({ a: x, b: 1 })   //@n:${Y_ID}`,
	"}",
	"",
	`eventsGeneric other(payload: Struct) {   //@n:${OTHER_ID}`,
	`    log({ msg: "hi" })   //@n:${LOG_ID}`,
	"}",
	"",
];
const BASE = BASE_LINES.join("\n");

/** Replace the single line carrying `anchorId` with `next` (scripted edit). */
function editAnchoredLine(
	text: string,
	anchorId: string,
	next: string,
): string {
	const lines = text.split("\n");
	const at = lines.findIndex((line) => line.endsWith(`//@n:${anchorId}`));
	if (at < 0) throw new Error(`no line anchored by ${anchorId}`);
	lines[at] = next;
	return lines.join("\n");
}

function insertAfterAnchoredLine(
	text: string,
	anchorId: string,
	inserted: string,
): string {
	const lines = text.split("\n");
	const at = lines.findIndex((line) => line.endsWith(`//@n:${anchorId}`));
	if (at < 0) throw new Error(`no line anchored by ${anchorId}`);
	lines.splice(at + 1, 0, inserted);
	return lines.join("\n");
}

function deleteAnchoredLine(text: string, anchorId: string): string {
	const lines = text.split("\n");
	const at = lines.findIndex((line) => line.endsWith(`//@n:${anchorId}`));
	if (at < 0) throw new Error(`no line anchored by ${anchorId}`);
	lines.splice(at, 1);
	return lines.join("\n");
}

function mergeOrThrow(input: {
	baseline: string;
	local: string;
	fresh: string;
}) {
	const result = mergeFlowScript(input);
	if (!result.ok) throw new Error(`merge failed: ${result.reason}`);
	return result;
}

describe("FlowScript unit segmentation", () => {
	test("units partition the text exactly (roundtrip)", () => {
		for (const text of [
			BASE,
			fixture("ttwctnp08u18sg2z6nmcqqak.anchored.flow"),
			fixture("bypaw6n2ksuvrw0kcaj14omz.anchored.flow"),
		]) {
			const { units, duplicateAnchorId } = segmentFlowScriptUnits(text);
			expect(duplicateAnchorId).toBeUndefined();
			expect(units.map((unit) => unit.text).join("\n")).toBe(text);
		}
	});

	test("anchored lines own the unanchored residue below them", () => {
		const { units } = segmentFlowScriptUnits(BASE);
		const byKey = new Map(units.map((unit) => [unit.anchorId, unit]));
		// Preamble = use block + interface, up to the first anchored line.
		expect(units[0].anchorId).toBeUndefined();
		expect(units[0].text).toBe(BASE_LINES.slice(0, 5).join("\n"));
		// Per-variable unit carries its trailing blank separator.
		expect(byKey.get(VAR_ID)?.text).toBe(BASE_LINES.slice(5, 7).join("\n"));
		// A section header line is its own unit; the section's closing brace
		// attaches to the LAST statement inside it.
		expect(byKey.get(MAIN_ID)?.text).toBe(BASE_LINES[7]);
		expect(byKey.get(Y_ID)?.text).toBe(BASE_LINES.slice(9, 12).join("\n"));
		// The final unit soaks up the trailing empty line.
		expect(byKey.get(LOG_ID)?.text).toBe(BASE_LINES.slice(13).join("\n"));
	});

	test("a text starting on an anchored line has no preamble unit", () => {
		const { units } = segmentFlowScriptUnits(
			`const x = now()   //@n:${X_ID}\nreturn x`,
		);
		expect(units).toHaveLength(1);
		expect(units[0].anchorId).toBe(X_ID);
	});

	test("flags duplicate anchor ids", () => {
		const { duplicateAnchorId } = segmentFlowScriptUnits(
			[`const a = 1   //@n:${X_ID}`, `const b = 2   //@n:${X_ID}`].join("\n"),
		);
		expect(duplicateAnchorId).toBe(X_ID);
	});
});

describe("FlowScript three-way merge", () => {
	test("a clean local buffer takes the fresh render verbatim (idempotence)", () => {
		const fresh = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = later()   //@n:${X_ID}`,
		);
		const result = mergeOrThrow({ baseline: BASE, local: BASE, fresh });
		expect(result.mergedText).toBe(fresh);
		expect(result.conflicts).toHaveLength(0);
		expect(result.stats.tookFresh).toBe(1);
		expect(result.remoteTouchedAnchorIds).toEqual([X_ID]);
	});

	test("an unchanged board keeps the local buffer verbatim", () => {
		const local = insertAfterAnchoredLine(BASE, X_ID, "    // my new note");
		const result = mergeOrThrow({ baseline: BASE, local, fresh: BASE });
		expect(result.mergedText).toBe(local);
		expect(result.conflicts).toHaveLength(0);
		expect(result.stats.tookLocal).toBe(1);
		expect(result.remoteTouchedAnchorIds).toEqual([]);
	});

	test("disjoint edits merge without conflicts, with a stats toastable summary", () => {
		const local = editAnchoredLine(
			BASE,
			LOG_ID,
			`    log({ msg: "edited locally" })   //@n:${LOG_ID}`,
		);
		const fresh = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = later()   //@n:${X_ID}`,
		);
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(0);
		expect(result.mergedText).toContain('log({ msg: "edited locally" })');
		expect(result.mergedText).toContain("const x = later()");
		expect(result.stats).toMatchObject({ tookFresh: 1, tookLocal: 1 });
	});

	test("the same unit changed on both sides becomes a both-changed conflict keeping the local block", () => {
		const local = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = mine()   //@n:${X_ID}`,
		);
		const fresh = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = theirs()   //@n:${X_ID}`,
		);
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(1);
		const conflict = result.conflicts[0];
		expect(conflict.kind).toBe("both-changed");
		expect(conflict.anchorId).toBe(X_ID);
		expect(conflict.localBlock).toContain("mine()");
		expect(conflict.freshBlock).toContain("theirs()");
		expect(result.mergedText).toContain("mine()");
		expect(result.mergedText).not.toContain("theirs()");
		// The conflict's line points at the unit inside mergedText.
		const mergedLines = result.mergedText.split("\n");
		expect(mergedLines[conflict.line - 1]).toContain("mine()");
	});

	test("identical edits on both sides converge without a conflict", () => {
		const edited = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = same()   //@n:${X_ID}`,
		);
		const result = mergeOrThrow({
			baseline: BASE,
			local: edited,
			fresh: edited,
		});
		expect(result.conflicts).toHaveLength(0);
		expect(result.mergedText).toBe(edited);
	});

	test("a unit deleted remotely while locally edited becomes a remote-deleted conflict kept in place", () => {
		const local = editAnchoredLine(
			BASE,
			Y_ID,
			`    const y = add({ a: x, b: 99 })   //@n:${Y_ID}`,
		);
		const fresh = deleteAnchoredLine(BASE, Y_ID);
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(1);
		expect(result.conflicts[0].kind).toBe("remote-deleted");
		expect(result.conflicts[0].anchorId).toBe(Y_ID);
		expect(result.conflicts[0].freshBlock).toBe("");
		// The kept local unit follows its local predecessor's (fresh) unit —
		// the deletion moved the structural residue into X's unit, so the Y
		// block lands after it, still ahead of the next section.
		const mergedLines = result.mergedText.split("\n");
		const xLine = mergedLines.findIndex((line) =>
			line.includes(`//@n:${X_ID}`),
		);
		const yLine = mergedLines.findIndex((line) => line.includes("b: 99"));
		const otherLine = mergedLines.findIndex((line) =>
			line.includes(`//@n:${OTHER_ID}`),
		);
		expect(yLine).toBeGreaterThan(xLine);
		expect(yLine).toBeLessThan(otherLine);
		expect(result.remoteTouchedAnchorIds).toContain(Y_ID);
		// Taking theirs (the deletion) restores the fresh render exactly.
		expect(
			resolveFlowScriptConflict(
				result.mergedText,
				result.conflicts[0],
				"theirs",
			),
		).toBe(fresh);
	});

	test("a local deletion of an untouched unit is preserved", () => {
		const local = deleteAnchoredLine(BASE, LOG_ID);
		const result = mergeOrThrow({ baseline: BASE, local, fresh: BASE });
		expect(result.conflicts).toHaveLength(0);
		expect(result.mergedText).toBe(local);
	});

	test("a unit deleted locally but changed remotely conflicts with an empty local block", () => {
		const local = deleteAnchoredLine(BASE, LOG_ID);
		const fresh = editAnchoredLine(
			BASE,
			LOG_ID,
			`    log({ msg: "fresh" })   //@n:${LOG_ID}`,
		);
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(1);
		expect(result.conflicts[0].kind).toBe("both-changed");
		expect(result.conflicts[0].localBlock).toBe("");
		// The fresh block stays visible so the conflict has a line to resolve on.
		expect(result.mergedText).toContain('log({ msg: "fresh" })');
	});

	test("new units in fresh are included", () => {
		const fresh = insertAfterAnchoredLine(
			BASE,
			X_ID,
			`    const z = twice(x)   //@n:${NEW_ID}`,
		);
		const local = editAnchoredLine(
			BASE,
			LOG_ID,
			`    log({ msg: "local" })   //@n:${LOG_ID}`,
		);
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(0);
		expect(result.mergedText).toContain("twice(x)");
		expect(result.mergedText).toContain('"local"');
		expect(result.stats.freshAdded).toBe(1);
		expect(result.remoteTouchedAnchorIds).toContain(NEW_ID);
	});

	test("local unanchored additions ride their unit through a merge", () => {
		const local = insertAfterAnchoredLine(BASE, Y_ID, "    return y");
		const fresh = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = later()   //@n:${X_ID}`,
		);
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(0);
		const mergedLines = result.mergedText.split("\n");
		const yLine = mergedLines.findIndex((line) =>
			line.includes(`//@n:${Y_ID}`),
		);
		expect(mergedLines[yLine + 1]).toBe("    return y");
		expect(result.mergedText).toContain("later()");
	});

	test("preamble changes on both sides conflict as one unit at line 1", () => {
		const local = BASE.replace("a: string;", "a: string;\n    b: int;");
		const fresh = BASE.replace("a: string;", "a: number;");
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(1);
		expect(result.conflicts[0].anchorId).toBeUndefined();
		expect(result.conflicts[0].line).toBe(1);
	});

	test("remote-only preamble changes flow in under local statement edits", () => {
		const local = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = mine()   //@n:${X_ID}`,
		);
		const fresh = BASE.replace("use std::*", "use std::*\nuse http::*");
		const result = mergeOrThrow({ baseline: BASE, local, fresh });
		expect(result.conflicts).toHaveLength(0);
		expect(result.mergedText).toContain("use http::*");
		expect(result.mergedText).toContain("mine()");
	});

	test("duplicate anchors in any input fail the merge (guard fallback)", () => {
		const broken = `${BASE}\nconst dup = 1   //@n:${X_ID}`;
		for (const input of [
			{ baseline: broken, local: BASE, fresh: BASE },
			{ baseline: BASE, local: broken, fresh: BASE },
			{ baseline: BASE, local: BASE, fresh: broken },
		]) {
			const result = mergeFlowScript(input);
			expect(result.ok).toBe(false);
		}
	});

	test("fixture-backed: disjoint edits on a real anchored render merge cleanly", () => {
		const baseline = fixture("ttwctnp08u18sg2z6nmcqqak.anchored.flow");
		const local = editAnchoredLine(
			baseline,
			"hz2lby9sw0zcg6e90b4etdhs",
			"    const date = tomorrow()   //@n:hz2lby9sw0zcg6e90b4etdhs",
		);
		const fresh = editAnchoredLine(
			baseline,
			"dcc9b9ioxr85bjr1t6kt0cyt",
			"    const database = open({ name: table, userScoped: false, batchSize: 500 })   //@n:dcc9b9ioxr85bjr1t6kt0cyt",
		);
		const result = mergeOrThrow({ baseline, local, fresh });
		expect(result.conflicts).toHaveLength(0);
		expect(result.mergedText).toContain("tomorrow()");
		expect(result.mergedText).toContain("batchSize: 500");
		expect(result.remoteTouchedAnchorIds).toEqual(["dcc9b9ioxr85bjr1t6kt0cyt"]);
	});
});

describe("FlowScript conflict resolution", () => {
	const local = editAnchoredLine(
		BASE,
		X_ID,
		`    const x = mine()   //@n:${X_ID}`,
	);
	const fresh = editAnchoredLine(
		BASE,
		X_ID,
		`    const x = theirs()   //@n:${X_ID}`,
	);
	const merged = mergeOrThrow({ baseline: BASE, local, fresh });
	const conflict = merged.conflicts[0];

	test("take theirs splices only the conflicted unit", () => {
		const resolved = resolveFlowScriptConflict(
			merged.mergedText,
			conflict,
			"theirs",
		);
		expect(resolved).toBe(fresh);
	});

	test("keep mine leaves the buffer untouched", () => {
		expect(resolveFlowScriptConflict(merged.mergedText, conflict, "mine")).toBe(
			merged.mergedText,
		);
	});

	test("keep mine on a locally deleted unit removes the fresh block", () => {
		const deletedLocal = deleteAnchoredLine(BASE, LOG_ID);
		const freshEdit = editAnchoredLine(
			BASE,
			LOG_ID,
			`    log({ msg: "fresh" })   //@n:${LOG_ID}`,
		);
		const result = mergeOrThrow({
			baseline: BASE,
			local: deletedLocal,
			fresh: freshEdit,
		});
		const resolved = resolveFlowScriptConflict(
			result.mergedText,
			result.conflicts[0],
			"mine",
		);
		expect(resolved).not.toContain(`//@n:${LOG_ID}`);
	});

	test("resolving units the user has since deleted appends theirs instead of failing", () => {
		const withoutUnit = deleteAnchoredLine(merged.mergedText, X_ID);
		const resolved = resolveFlowScriptConflict(withoutUnit, conflict, "theirs");
		expect(resolved).toContain("theirs()");
	});
});

describe("remote-touched apply-preview overlap", () => {
	test("intersects locally edited anchors with the remote-touched set", () => {
		expect(
			intersectRemoteTouched(new Set([X_ID, LOG_ID]), [Y_ID, LOG_ID]),
		).toEqual([LOG_ID]);
		expect(intersectRemoteTouched(new Set(), [X_ID])).toEqual([]);
	});
});

describe("conflict CodeLens derivation", () => {
	test("one lens per conflict at the unit's current anchor line; vanished anchors render none", () => {
		const local = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = mine()   //@n:${X_ID}`,
		);
		const fresh = editAnchoredLine(
			BASE,
			X_ID,
			`    const x = theirs()   //@n:${X_ID}`,
		);
		const merged = mergeOrThrow({ baseline: BASE, local, fresh });
		const index = parseFlowScriptAnchors(merged.mergedText);
		const lenses = deriveFlowScriptConflictLenses(merged.conflicts, index);
		expect(lenses).toHaveLength(1);
		expect(lenses[0].conflictIndex).toBe(0);
		expect(merged.mergedText.split("\n")[lenses[0].line - 1]).toContain(
			"mine()",
		);

		const withoutAnchor = deriveFlowScriptConflictLenses(
			merged.conflicts,
			parseFlowScriptAnchors("nothing anchored here"),
		);
		expect(withoutAnchor).toHaveLength(0);
	});
});
