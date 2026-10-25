import { describe, expect, it, test } from "bun:test";
import type { IChatWidget, IPlanStep } from "./chat-db";
import {
	type InlineChatSegment,
	buildInlineSegments,
	joinContentText,
	safeSplitOffset,
	unanchoredWidgets,
} from "./inline-segments";

function step(id: string, content_offset?: number): IPlanStep {
	return { id, title: `Using ${id}`, status: "done", content_offset };
}

function widget(id: string, content_offset?: number): IChatWidget {
	return {
		instance_id: id,
		widget_id: "w",
		surface_id: id,
		component: {},
		content_offset,
	};
}

function outline(segments: InlineChatSegment[] | null) {
	return segments?.map(
		(s) =>
			s.text ??
			(s.steps && `steps:${s.steps.map((x) => x.id).join(",")}`) ??
			(s.widgets &&
				`widgets:${s.widgets.map((x) => x.instance_id).join(",")}`) ??
			`content:${s.content?.key}`,
	);
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

describe("buildInlineSegments with widgets and host content", () => {
	it("stays null while no widget is anchored, and step output is unchanged by them", () => {
		expect(buildInlineSegments("hello", [], [widget("w")])).toBeNull();
		const steps = [step("a", 2), step("b")];
		expect(buildInlineSegments("hello", steps, [widget("w")])).toEqual(
			buildInlineSegments("hello", steps),
		);
	});

	it("places an anchored widget between the text it interrupted", () => {
		const segments = buildInlineSegments(
			"before after",
			[],
			[widget("map", 6)],
		);
		expect(outline(segments)).toEqual(["before", "widgets:map", " after"]);
		expect(segments?.[1]?.key).toBe("widgets-map");
	});

	it("leaves unanchored steps to the legacy block and unanchored widgets out", () => {
		const segments = buildInlineSegments(
			"text here",
			[step("legacy")],
			[widget("loose"), widget("pinned", 4)],
		);
		expect(outline(segments)).toEqual(["text", "widgets:pinned", " here"]);
	});

	it("renders steps before widgets at one offset and groups those widgets", () => {
		const segments = buildInlineSegments(
			"intro",
			[step("tool", 5)],
			[widget("first", 5), widget("second", 5)],
		);
		expect(outline(segments)).toEqual([
			"intro",
			"steps:tool",
			"widgets:first,second",
		]);
	});

	it("gives every host entry its own segment after widgets at the same offset", () => {
		const segments = buildInlineSegments(
			"ab",
			[],
			[widget("w", 1)],
			[
				{ key: "page", offset: 1 },
				{ key: "chat", offset: 1 },
			],
		);
		expect(outline(segments)).toEqual([
			"a",
			"widgets:w",
			"content:page",
			"content:chat",
			"b",
		]);
		expect(segments?.map((s) => s.key)).toEqual([
			"text-0",
			"widgets-w",
			"content-page",
			"content-chat",
			"text-1",
		]);
	});

	it("keeps a widget anchored inside an open fence after the fence", () => {
		const text = "intro\n```js\nconst a = 1;\n```\ntail";
		const segments = buildInlineSegments(
			text,
			[],
			[widget("w", text.indexOf("const"))],
		);
		expect(segments?.[0]?.text?.split("```")).toHaveLength(3);
		expect(outline(segments)?.slice(1)).toEqual(["widgets:w", "tail"]);
	});
});

describe("unanchoredWidgets", () => {
	test("returns the same array when no widget is anchored", () => {
		const widgets = [widget("a"), widget("b")];
		expect(unanchoredWidgets(widgets)).toBe(widgets);
		expect(unanchoredWidgets(undefined)).toBeUndefined();
	});

	test("drops the anchored ones", () => {
		expect(
			unanchoredWidgets([widget("a", 0), widget("b")])?.map(
				(w) => w.instance_id,
			),
		).toEqual(["b"]);
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
