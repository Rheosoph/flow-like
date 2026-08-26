import { describe, expect, test } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import {
	anchorAtLine,
	anchorAtOrAbove,
	parseFlowScriptAnchors,
} from "./flowscript-anchors";

const FIXTURE_DIR = join(import.meta.dir, "../../../../../tests/ast");

function fixture(name: string): string {
	return readFileSync(join(FIXTURE_DIR, name), "utf8");
}

describe("FlowScript anchor parsing", () => {
	test("maps trailing statement anchors to their entity", () => {
		const index = parseFlowScriptAnchors(
			[
				"const date2 = now()   //@n:j37yxnwykc08y10reikmija1",
				'const files: NodeDBConnection = {"cache_key":""}   //@v:wjapgelphq6vh3pr4jtm403n',
				"function constructPrompt(prompt: string): (out: string) {   //@l:olhizg2b8s6seeuntnn1ni4o",
			].join("\n"),
		);
		expect(anchorAtLine(index, 1)).toEqual({
			id: "j37yxnwykc08y10reikmija1",
			kind: "node",
			line: 1,
			column: 23,
			endColumn: 52,
		});
		expect(anchorAtLine(index, 2)?.kind).toBe("variable");
		expect(anchorAtLine(index, 3)?.kind).toBe("layer");
		expect(index.firstLineById.get("olhizg2b8s6seeuntnn1ni4o")).toBe(3);
	});

	test("a branch line keeps its anchor after a pin-name comment", () => {
		const index = parseFlowScriptAnchors(
			"if (exists({ path: pathOut5 })) { // exec_out_exists   //@n:cqrbjotyj6ppoatuxve6ewzl",
		);
		const anchor = anchorAtLine(index, 1);
		expect(anchor?.id).toBe("cqrbjotyj6ppoatuxve6ewzl");
		expect(anchor?.kind).toBe("node");
	});

	test("ignores anchor-shaped text inside string and template literals", () => {
		const index = parseFlowScriptAnchors(
			[
				'const fake = "not an anchor //@n:aaaaaaaaaaaaaaaaaaaaaaaa"',
				"const tpl = `also not one //@v:bbbbbbbbbbbbbbbbbbbbbbbb",
				"still template text //@l:cccccccccccccccccccccccc",
				"`   //@v:realvariableanchor0001",
			].join("\n"),
		);
		expect(index.anchors).toHaveLength(1);
		expect(index.anchors[0]).toMatchObject({
			id: "realvariableanchor0001",
			kind: "variable",
			line: 4,
		});
	});

	test("template expressions may carry comments and nested braces", () => {
		const index = parseFlowScriptAnchors(
			[
				"const text = `head ${ set({ a: { b: 1 } }).out } tail",
				"more template //@n:dddddddddddddddddddddddd",
				"` //@n:eeeeeeeeeeeeeeeeeeeeeeee",
			].join("\n"),
		);
		expect(index.anchors.map((anchor) => anchor.id)).toEqual([
			"eeeeeeeeeeeeeeeeeeeeeeee",
		]);
	});

	test("an anchor must terminate the line", () => {
		const index = parseFlowScriptAnchors(
			"const x = 1   //@n:ffffffffffffffffffffffff trailing words",
		);
		expect(index.anchors).toHaveLength(0);
	});

	test("anchorAtOrAbove walks up over un-anchored block lines", () => {
		const index = parseFlowScriptAnchors(
			[
				"if (flag) { // exec_out   //@n:branchnodeanchor00000001",
				"    doWork()   //@n:worknodeanchor0000000001",
				"} else {",
				"",
			].join("\n"),
		);
		expect(anchorAtLine(index, 3)).toBeUndefined();
		expect(anchorAtOrAbove(index, 3)?.id).toBe("worknodeanchor0000000001");
		expect(anchorAtOrAbove(index, 4, 0)).toBeUndefined();
	});

	test("firstLineById keeps the first occurrence", () => {
		const index = parseFlowScriptAnchors(
			[
				"alpha()   //@n:duplicated00000000000001",
				"beta()   //@n:duplicated00000000000001",
			].join("\n"),
		);
		expect(index.firstLineById.get("duplicated00000000000001")).toBe(1);
		expect(index.anchors).toHaveLength(2);
	});
});

describe("FlowScript anchor parsing — committed fixtures", () => {
	const fixtures = readdirSync(FIXTURE_DIR).filter((name) =>
		name.endsWith(".anchored.flow"),
	);

	test("fixtures are present", () => {
		expect(fixtures.length).toBeGreaterThan(0);
	});

	for (const name of fixtures) {
		test(`every trailing anchor in ${name} is indexed`, () => {
			const text = fixture(name);
			const index = parseFlowScriptAnchors(text);
			const lines = text.split("\n");
			// Count expected anchors the dumb way: lines ending in an anchor whose
			// final anchor token is preceded (transitively) by real code, i.e. lines
			// that are statements. In the committed fixtures every anchor-terminated
			// line is a statement line, so the naive count is exact.
			const expected = lines.filter((line) =>
				/\/\/@[nvl]:[A-Za-z0-9_-]+\s*$/.test(line),
			).length;
			expect(index.anchors.length).toBe(expected);
			for (const anchor of index.anchors) {
				const line = lines[anchor.line - 1];
				expect(line.slice(anchor.column - 1, anchor.endColumn - 1)).toBe(
					`//@${anchor.kind === "node" ? "n" : anchor.kind === "variable" ? "v" : "l"}:${anchor.id}`,
				);
			}
		});
	}

	test("known fixture anchors resolve to the right lines", () => {
		const index = parseFlowScriptAnchors(
			fixture("ttwctnp08u18sg2z6nmcqqak.anchored.flow"),
		);
		expect(index.firstLineById.get("ovthhqty1jphkp6pfalvyjid")).toBe(168);
		expect(anchorAtLine(index, 183)).toMatchObject({
			kind: "layer",
			id: "olhizg2b8s6seeuntnn1ni4o",
		});
		expect(anchorAtLine(index, 225)).toMatchObject({
			kind: "node",
			id: "cqrbjotyj6ppoatuxve6ewzl",
		});
		// The giant single-line string prompt still anchors as a variable.
		expect(anchorAtLine(index, 173)).toMatchObject({
			kind: "variable",
			id: "jsfpl1h011p2g48n8e2cvy75",
		});
	});
});
