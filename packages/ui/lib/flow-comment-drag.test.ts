import { describe, expect, test } from "bun:test";
import type { Node } from "@xyflow/react";
import { followAnchor, startCommentDrag } from "./flow-comment-drag";

function box(
	id: string,
	type: string,
	x: number,
	y: number,
	width: number,
	height: number,
	extra: Partial<Node> = {},
): Node {
	return {
		id,
		type,
		position: { x, y },
		measured: { width, height },
		data: {},
		...extra,
	};
}

const comment = box("comment", "commentNode", 0, 0, 400, 300);

describe("startCommentDrag", () => {
	test("carries nodes whose centre the comment covers", () => {
		const inside = box("inside", "node", 50, 50, 100, 60);
		const edge = box("edge", "node", 350, 100, 100, 60);
		const outside = box("outside", "node", 380, 100, 100, 60);
		const session = startCommentDrag(
			comment,
			[comment],
			[comment, inside, edge, outside],
		);
		expect([...(session?.passengers.keys() ?? [])].sort()).toEqual([
			"edge",
			"inside",
		]);
	});

	test("follows nested comments to what they cover", () => {
		const nested = box("nested", "commentNode", 250, 150, 250, 200);
		const underNested = box("under-nested", "node", 430, 300, 40, 20);
		const session = startCommentDrag(
			comment,
			[comment],
			[comment, nested, underNested],
		);
		expect([...(session?.passengers.keys() ?? [])].sort()).toEqual([
			"nested",
			"under-nested",
		]);
	});

	test("leaves locked comments, dragged nodes and transient nodes alone", () => {
		const locked = box("locked", "commentNode", 20, 20, 100, 100, {
			draggable: false,
		});
		const selected = box("selected", "node", 50, 150, 100, 60);
		const upload = box("upload", "uploadPlaceholderNode", 200, 100, 50, 50);
		const hidden = box("hidden", "node", 200, 200, 50, 50, { hidden: true });
		const session = startCommentDrag(
			comment,
			[comment, selected],
			[comment, locked, selected, upload, hidden],
		);
		expect(session).toBeUndefined();
	});

	test("only comments carry passengers", () => {
		const layer = box("layer", "layerNode", 0, 0, 400, 300);
		const node = box("node", "node", 50, 50, 100, 60);
		expect(startCommentDrag(layer, [layer], [layer, node])).toBeUndefined();
	});
});

describe("followAnchor", () => {
	test("keeps every passenger's offset to the anchor", () => {
		const inside = box("inside", "node", 50, 50, 100, 60);
		const session = startCommentDrag(comment, [comment], [comment, inside]);
		if (!session) throw new Error("expected a comment drag session");
		const moved = followAnchor(session, [
			{ id: "comment", position: { x: 120, y: -40 } },
		]);
		expect(moved).toEqual([{ id: "inside", x: 170, y: 10 }]);
	});

	test("does nothing once the anchor left the drag", () => {
		const inside = box("inside", "node", 50, 50, 100, 60);
		const session = startCommentDrag(comment, [comment], [comment, inside]);
		if (!session) throw new Error("expected a comment drag session");
		expect(followAnchor(session, [])).toEqual([]);
	});
});
