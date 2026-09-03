import { describe, expect, it, test } from "bun:test";
import type { IPlanStep } from "./chat-db";
import {
	buildInlineSegments,
	joinContentText,
	safeSplitOffset,
} from "./inline-segments";

function step(id: string, content_offset?: number): IPlanStep {
	return { id, title: `Using ${id}`, status: "done", content_offset };
}

describe("buildInlineSegments", () => {
	it("returns null without steps or anchors so callers keep the legacy layout", () => {
		expect(buildInlineSegments("hello", [])).toBeNull();
		expect(buildInlineSegments("hello", [step("a"), step("b")])).toBeNull();
	});

	it("interleaves text around an action anchored mid-reply", () => {
		const segments = buildInlineSegments("before\nafter", [step("search", 6)]);
		expect(segments?.map((s) => s.text ?? s.steps?.map((x) => x.id))).toEqual([
			"before",
			["search"],
			"\nafter",
		]);
	});

	it("groups actions sharing an anchor and keeps their original order", () => {
		const segments = buildInlineSegments("intro", [
			step("first", 5),
			step("second", 5),
		]);
		expect(segments?.[0]?.text).toBe("intro");
		expect(segments?.[1]?.steps?.map((s) => s.id)).toEqual(["first", "second"]);
		expect(segments).toHaveLength(2);
	});

	it("orders actions by anchor, not by array position", () => {
		const segments = buildInlineSegments("aaaabbbb", [
			step("late", 8),
			step("early", 4),
		]);
		expect(segments?.map((s) => s.text ?? s.steps?.[0]?.id)).toEqual([
			"aaaa",
			"early",
			"bbbb",
			"late",
		]);
	});

	it("clamps out-of-range anchors instead of producing empty or negative slices", () => {
		const segments = buildInlineSegments("short", [
			step("past-end", 999),
			step("negative", -5),
		]);
		expect(segments?.map((s) => s.text ?? s.steps?.[0]?.id)).toEqual([
			"negative",
			"short",
			"past-end",
		]);
	});

	it("treats a message with only unanchored steps as legacy, but keeps partial anchors", () => {
		const segments = buildInlineSegments("text here", [
			step("anchored", 4),
			step("unanchored"),
		]);
		// The unanchored step falls back to offset 0 and leads the reply.
		expect(segments?.map((s) => s.text ?? s.steps?.[0]?.id)).toEqual([
			"unanchored",
			"text",
			"anchored",
			" here",
		]);
	});

	it("drops whitespace-only slices", () => {
		const segments = buildInlineSegments("a\n\n   \nb", [step("mid", 2)]);
		expect(segments?.filter((s) => s.text)?.map((s) => s.text?.trim())).toEqual(
			["a", "b"],
		);
	});

	it("does not split inside an unterminated code fence", () => {
		const text = "intro\n```js\nconst a = 1;\n```\ntail";
		const anchorInsideFence = text.indexOf("const");
		const segments = buildInlineSegments(text, [
			step("tool", anchorInsideFence),
		]);
		const first = segments?.[0]?.text ?? "";
		// The whole fence stays in one segment; the action lands after its closing line.
		expect(first).toContain("```js");
		expect(first.match(/```/g)).toHaveLength(2);
		expect(segments?.[1]?.steps?.[0]?.id).toBe("tool");
		expect(segments?.[2]?.text).toBe("tail");
	});
});

describe("safeSplitOffset", () => {
	it("leaves offsets outside fences untouched", () => {
		expect(safeSplitOffset("plain text", 5)).toBe(5);
		expect(safeSplitOffset("a\n```\nx\n```\nb", 13)).toBe(13);
	});

	it("pushes an offset inside an open fence past the closing line", () => {
		const text = "```\ncode\n```\nafter";
		expect(safeSplitOffset(text, 6)).toBe(text.indexOf("after"));
	});

	it("pushes to the end when the fence never closes (still streaming)", () => {
		const text = "```\ncode in flight";
		expect(safeSplitOffset(text, 6)).toBe(text.length);
	});

	it("never splits inside the fence marker itself", () => {
		const text = "text```\ncode\n```\nend";
		// Offset 5 sits between the backticks of the opening marker.
		expect(safeSplitOffset(text, 5)).toBe(4);
	});
});

describe("joinContentText", () => {
	test("returns string content untouched", () => {
		expect(joinContentText("plain **text**")).toBe("plain **text**");
	});

	test("concatenates text parts raw and skips media parts", () => {
		expect(
			joinContentText([
				{ type: "image_url", image_url: { url: "https://x/a.png" } },
				{ type: "text", text: "Hel" },
				{ type: "text", text: "lo" },
			] as never),
		).toBe("Hello");
	});

	test("treats missing content as empty", () => {
		expect(joinContentText(undefined)).toBe("");
		expect(joinContentText(null)).toBe("");
	});
});
