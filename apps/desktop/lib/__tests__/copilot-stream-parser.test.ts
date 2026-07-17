import {
	appendBoundedStreamDetail,
	createCopilotStreamParser,
} from "@flow-like/flow-like-ui/components/flowpilot/copilot-stream-parser";
import { describe, expect, test } from "vitest";

describe("copilot stream parser", () => {
	test("preserves chronological order across different control-frame types", () => {
		const parser = createCopilotStreamParser();
		const events = parser.push(
			[
				"before ",
				'<tool_start>{"tool_call_id":"tool-1"}</tool_start>',
				" between ",
				'<components>[{"id":"root"}]</components>',
				'<tool_end>{"tool_call_id":"tool-1","status":"done"}</tool_end>',
				" after",
			].join(""),
		);

		expect(events.map((event) => event.type)).toEqual([
			"text",
			"tool_start",
			"text",
			"components",
			"tool_end",
			"text",
		]);
		expect(events.filter((event) => event.type === "text")).toEqual([
			{ type: "text", text: "before " },
			{ type: "text", text: " between " },
			{ type: "text", text: " after" },
		]);
	});

	test("buffers boundaries inside an opening tag, frame body, and closing tag", () => {
		const frame =
			'<tool_start>{"tool_call_id":"tool-1","arguments":{"safe":true}}</tool_start>';
		const splits = [
			frame.indexOf("<tool_start>") + "<tool_".length,
			frame.indexOf('"safe"') + 3,
			frame.indexOf("</tool_start>") + "</tool_".length,
		];

		for (const splitAt of splits) {
			const parser = createCopilotStreamParser();
			expect(parser.push(frame.slice(0, splitAt))).toEqual([]);
			expect(parser.push(frame.slice(splitAt))).toEqual([
				expect.objectContaining({
					type: "tool_start",
					data: {
						tool_call_id: "tool-1",
						arguments: { safe: true },
					},
				}),
			]);
			expect(parser.flush()).toEqual([]);
		}
	});

	test("emits definite text while retaining a possible split opening tag", () => {
		const parser = createCopilotStreamParser();

		expect(parser.push("hello <tool_sta")).toEqual([
			{ type: "text", text: "hello " },
		]);
		expect(
			parser.push('rt>{"tool_call_id":"tool-2"}</tool_start> goodbye'),
		).toEqual([
			expect.objectContaining({
				type: "tool_start",
				data: { tool_call_id: "tool-2" },
			}),
			{ type: "text", text: " goodbye" },
		]);
	});

	test("flushes an incomplete control frame as literal assistant text", () => {
		const parser = createCopilotStreamParser();
		const incomplete = '<tool_end>{"tool_call_id":"tool-3"}';

		expect(parser.push(incomplete)).toEqual([]);
		expect(parser.flush()).toEqual([{ type: "text", text: incomplete }]);
	});

	test("reset discards a buffered partial frame", () => {
		const parser = createCopilotStreamParser();
		expect(parser.push("stale <components>[")).toEqual([
			{ type: "text", text: "stale " },
		]);
		parser.reset();
		expect(parser.push("fresh")).toEqual([{ type: "text", text: "fresh" }]);
	});

	test("bounds and discards an oversized incomplete control frame", () => {
		const parser = createCopilotStreamParser({ maxBufferedChars: 256 });
		expect(parser.push(`<components>${"x".repeat(300)}`)).toEqual([]);
		expect(parser.bufferedLength()).toBeLessThan(32);

		// The oversized payload is omitted, parsing resumes after a split closing tag.
		expect(parser.push("</compo")).toEqual([]);
		expect(parser.push("nents>after")).toEqual([
			{ type: "text", text: "after" },
		]);
		expect(parser.bufferedLength()).toBe(0);
	});

	test("keeps only recent progress detail within the configured bound", () => {
		const detail = appendBoundedStreamDetail("old".repeat(100), "latest", 80);
		expect(detail.length).toBe(80);
		expect(detail).toContain("earlier progress truncated");
		expect(detail.endsWith("latest")).toBe(true);
	});
});
