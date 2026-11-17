import { describe, expect, it } from "bun:test";
import { type IComment, ICommentType } from "../schema/flow/board";
import type { INode } from "../schema/flow/node";
import { commentLabel, pinAt, pinLabel } from "./board-targets";

function comment(content: string, type = ICommentType.Text): IComment {
	return {
		id: "c1",
		comment_type: type,
		content,
		coordinates: [0, 0, 0],
		timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	};
}

describe("board den targets", () => {
	it("names a comment by its first line of text", () => {
		expect(
			commentLabel(comment("\n## Retry **twice** on 429\nthen give up")),
		).toBe("Retry twice on 429");
		expect(commentLabel(comment("See [the docs](https://x.y) first"))).toBe(
			"See the docs first",
		);
		expect(commentLabel(comment("x".repeat(80)))).toBe(`${"x".repeat(59)}…`);
	});

	it("names a comment without text by its kind", () => {
		expect(commentLabel(comment("  ", ICommentType.Image))).toBe("Image");
	});

	it("finds the nearest pin within the radius", () => {
		const handles = [
			{ id: "in", x: 0, y: 30, width: 10, height: 10 },
			{ id: "out", x: 190, y: 30, width: 10, height: 10 },
			{ id: null, x: 0, y: 60, width: 10, height: 10 },
		];
		expect(pinAt(handles, { x: 8, y: 37 }, 12)?.id).toBe("in");
		expect(pinAt(handles, { x: 100, y: 37 }, 12)).toBeNull();
		expect(pinAt(handles, { x: 5, y: 65 }, 12)).toBeNull();
	});

	it("names a pin with its node", () => {
		const node = {
			friendly_name: "HTTP Request",
			name: "http_request",
			pins: { p1: { friendly_name: "URL", name: "url" } },
		} as unknown as INode;
		expect(pinLabel(node, "p1")).toBe("HTTP Request › URL");
		expect(pinLabel(node, "missing")).toBeUndefined();
	});
});
