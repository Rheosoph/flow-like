import { describe, expect, test } from "bun:test";
import {
	type SchemaLayoutEdge,
	type SchemaLayoutNode,
	layoutSchema,
} from "./ontology-schema-layout";

function box(id: string, height = 100): SchemaLayoutNode {
	return { id, width: 200, height };
}

function link(source: string, target: string, id?: string): SchemaLayoutEdge {
	return { id, source, target };
}

function columnsOf(positions: Map<string, { x: number }>) {
	return [...new Set([...positions.values()].map((point) => point.x))].sort(
		(a, b) => a - b,
	);
}

const archive = {
	nodes: [box("article"), box("entity"), box("image"), box("face")],
	edges: [
		link("article", "entity", "mentions"),
		link("article", "image", "has_image"),
		link("article", "face", "has_face"),
		link("image", "face", "shows"),
	],
};

describe("layoutSchema", () => {
	test("ranks objects left to right along their relationships", () => {
		const { positions } = layoutSchema(archive.nodes, archive.edges);
		const x = (id: string) => positions.get(id)?.x ?? Number.NaN;
		expect(x("article")).toBeLessThan(x("image"));
		expect(x("image")).toBe(x("entity"));
		expect(x("image")).toBeLessThan(x("face"));
		expect(columnsOf(positions)).toHaveLength(3);
	});

	test("reserves a lane clear of every card for a link that skips a column", () => {
		const { positions, lanes } = layoutSchema(archive.nodes, archive.edges);
		expect([...lanes.keys()]).toEqual(["has_face"]);
		const lane = lanes.get("has_face");
		const image = positions.get("image");
		expect(lane?.x).toBe((image?.x ?? 0) + 100);
		for (const [id, point] of positions) {
			const insideX =
				(lane?.x ?? 0) > point.x && (lane?.x ?? 0) < point.x + 200;
			const insideY =
				(lane?.y ?? 0) > point.y && (lane?.y ?? 0) < point.y + 100;
			expect(`${id}:${insideX && insideY}`).toBe(`${id}:false`);
		}
	});

	test("never overlaps objects within a column", () => {
		const nodes = [
			box("hub", 60),
			box("a", 180),
			box("b", 90),
			box("c", 140),
			box("d", 60),
		];
		const { positions } = layoutSchema(
			nodes,
			["a", "b", "c", "d"].map((id) => link("hub", id)),
		);
		const column = nodes
			.filter((node) => node.id !== "hub")
			.map((node) => ({
				top: positions.get(node.id)?.y ?? 0,
				bottom: (positions.get(node.id)?.y ?? 0) + node.height,
			}))
			.sort((a, b) => a.top - b.top);
		for (let index = 1; index < column.length; index += 1) {
			expect(column[index].top).toBeGreaterThanOrEqual(
				column[index - 1].bottom,
			);
		}
	});

	test("terminates on cycles and still separates their members", () => {
		const { positions } = layoutSchema(
			[box("a"), box("b"), box("c")],
			[link("a", "b"), link("b", "a"), link("b", "c"), link("c", "a")],
		);
		expect(positions.size).toBe(3);
		expect(positions.get("a")?.x).not.toBe(positions.get("b")?.x);
	});

	test("pulls a pure source next to the object it points at", () => {
		const { positions } = layoutSchema(
			[box("a"), box("b"), box("c"), box("d")],
			[link("a", "b"), link("b", "c"), link("d", "c")],
		);
		expect(positions.get("d")?.x).toBe(positions.get("b")?.x);
	});

	test("puts unlinked objects in a grid below the linked ones", () => {
		const nodes = [box("a"), box("b"), box("lonely"), box("alone")];
		const { positions } = layoutSchema(nodes, [link("a", "b")]);
		const linkedBottom = Math.max(
			(positions.get("a")?.y ?? 0) + 100,
			(positions.get("b")?.y ?? 0) + 100,
		);
		expect(positions.get("lonely")?.y ?? 0).toBeGreaterThan(linkedBottom);
		expect(positions.get("alone")?.y).toBe(positions.get("lonely")?.y);
	});

	test("ignores self-references and edges to unknown objects", () => {
		const { positions, lanes } = layoutSchema(
			[box("a"), box("b")],
			[link("a", "a"), link("a", "ghost")],
		);
		expect(positions.get("a")).toEqual({ x: 0, y: 0 });
		expect(positions.get("b")?.y).toBe(0);
		expect(lanes.size).toBe(0);
	});

	test("is deterministic for the same input", () => {
		const nodes = ["a", "b", "c", "d", "e", "f"].map((id) => box(id));
		const edges = [
			link("a", "c"),
			link("b", "c"),
			link("c", "d"),
			link("a", "e"),
			link("e", "f"),
			link("f", "a"),
			link("a", "d"),
		];
		const first = layoutSchema(nodes, edges);
		const second = layoutSchema(nodes, edges);
		expect([...first.positions]).toEqual([...second.positions]);
		expect([...first.lanes]).toEqual([...second.lanes]);
	});
});
