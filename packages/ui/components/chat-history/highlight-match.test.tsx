import { describe, expect, test } from "bun:test";
import type { ReactElement } from "react";
import { highlightMatch } from "./highlight-match";

/** Flatten the rendered nodes into `[plain, {mark}, plain, …]` for assertion. */
function parts(node: React.ReactNode): string[] {
	if (typeof node === "string") return [node];
	if (!Array.isArray(node)) return [];
	return node.map((part) =>
		typeof part === "string"
			? part
			: `{${(part as ReactElement<{ children: string }>).props.children}}`,
	);
}

describe("highlightMatch", () => {
	test("marks every occurrence, not every other one", () => {
		// The obvious implementation re-tests each part with a /g regex, whose stateful lastIndex
		// makes it skip alternating matches. Repeated identical terms catch that.
		expect(parts(highlightMatch("chat chat chat", "chat"))).toEqual([
			"",
			"{chat}",
			" ",
			"{chat}",
			" ",
			"{chat}",
			"",
		]);
	});

	test("matches case-insensitively and keeps the original casing", () => {
		expect(parts(highlightMatch("Board Copilot", "board"))).toEqual([
			"",
			"{Board}",
			" Copilot",
		]);
	});

	test("highlights each term independently of the order typed", () => {
		expect(
			parts(highlightMatch("deploy the pipeline", "pipeline deploy")),
		).toEqual(["", "{deploy}", " the ", "{pipeline}", ""]);
	});

	test("treats regex metacharacters as literal text", () => {
		expect(parts(highlightMatch("cost (v2) report", "(v2)"))).toEqual([
			"cost ",
			"{(v2)}",
			" report",
		]);
	});

	test("returns the plain string when there is nothing to mark", () => {
		expect(highlightMatch("untouched", "")).toBe("untouched");
		expect(highlightMatch("untouched", "   ")).toBe("untouched");
		expect(highlightMatch("untouched", "missing")).toBe("untouched");
	});
});
