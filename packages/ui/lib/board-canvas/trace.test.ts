import { describe, expect, test } from "bun:test";
import type { Scene, SceneEdge, SceneNode } from "./scene";
import {
	NO_TRACE,
	WireIndex,
	sameAnchor,
	sameEndpoint,
	wireEmphasis,
} from "./trace";

function node(id: string, kind: SceneNode["kind"] = "standard"): SceneNode {
	return {
		id,
		x: 0,
		y: 0,
		width: kind === "reroute" ? 16 : 150,
		height: kind === "reroute" ? 12 : 60,
		label: id,
		kind,
		selected: false,
		dimmed: false,
		pins: [
			{
				id: "in",
				direction: "input",
				x: 0,
				y: 6,
				label: "In",
				color: [250, 100, 120, 255],
				kind: "data",
			},
			{
				id: "out",
				direction: "output",
				x: 16,
				y: 6,
				label: "Out",
				color: [250, 100, 120, 255],
				kind: "data",
			},
		],
	};
}

function wire(id: string, source: string, target: string): SceneEdge {
	return {
		id,
		source: { nodeId: source, pinId: "out" },
		target: { nodeId: target, pinId: "in" },
		kind: "data",
		selected: false,
		color: [250, 100, 120, 255],
	};
}

/** a → r1 → r2 → { b, r3 → c }, d → a, and a dangling reroute r4 after e. */
function board(): Scene {
	return {
		nodes: [
			node("a"),
			node("b"),
			node("c"),
			node("d"),
			node("e"),
			node("r1", "reroute"),
			node("r2", "reroute"),
			node("r3", "reroute"),
			node("r4", "reroute"),
		],
		edges: [
			wire("a-r1", "a", "r1"),
			wire("r1-r2", "r1", "r2"),
			wire("r2-b", "r2", "b"),
			wire("r2-r3", "r2", "r3"),
			wire("r3-c", "r3", "c"),
			wire("d-a", "d", "a"),
			wire("e-r4", "e", "r4"),
		],
	};
}

const sorted = (values: readonly string[]) => [...values].sort();
const ends = (trace?: { ends: { nodeId: string; pinId: string }[] }) =>
	sorted(trace?.ends.map((end) => `${end.nodeId}.${end.pinId}`) ?? []);

describe("WireIndex.fromPin", () => {
	test("follows an output through every reroute to the pins at the far ends", () => {
		const trace = new WireIndex(board()).fromPin({ nodeId: "a", pinId: "out" });
		expect(sorted(trace?.edges ?? [])).toEqual(
			sorted(["a-r1", "r1-r2", "r2-b", "r2-r3", "r3-c"]),
		);
		expect(sorted(trace?.reroutes ?? [])).toEqual(["r1", "r2", "r3"]);
		expect(ends(trace)).toEqual(["b.in", "c.in"]);
	});

	test("an input follows its wires upstream only", () => {
		const index = new WireIndex(board());
		const upstream = index.fromPin({ nodeId: "c", pinId: "in" });
		expect(sorted(upstream?.edges ?? [])).toEqual(
			sorted(["r3-c", "r2-r3", "r1-r2", "a-r1"]),
		);
		expect(ends(upstream)).toEqual(["a.out"]);
		expect(ends(index.fromPin({ nodeId: "a", pinId: "in" }))).toEqual([
			"d.out",
		]);
	});

	test("a reroute's pin traces the whole path through it", () => {
		const trace = new WireIndex(board()).fromPin({ nodeId: "r2", pinId: "in" });
		expect(sorted(trace?.edges ?? [])).toEqual(
			sorted(["a-r1", "r1-r2", "r2-b", "r2-r3", "r3-c"]),
		);
		expect(ends(trace)).toEqual(["a.out", "b.in", "c.in"]);
	});

	test("a reroute with nothing beyond it ends the path", () => {
		const trace = new WireIndex(board()).fromPin({ nodeId: "e", pinId: "out" });
		expect(trace?.edges).toEqual(["e-r4"]);
		expect(ends(trace)).toEqual(["r4.in"]);
	});

	test("cycles through reroutes terminate", () => {
		const scene: Scene = {
			nodes: [node("a"), node("r1", "reroute"), node("r2", "reroute")],
			edges: [
				wire("a-r1", "a", "r1"),
				wire("r1-r2", "r1", "r2"),
				wire("r2-r1", "r2", "r1"),
			],
		};
		const trace = new WireIndex(scene).fromPin({ nodeId: "a", pinId: "out" });
		expect(sorted(trace?.edges ?? [])).toEqual(["a-r1", "r1-r2", "r2-r1"]);
		expect(trace?.ends).toEqual([]);
	});

	test("unwired or unknown pins have no trace", () => {
		const index = new WireIndex(board());
		expect(index.fromPin({ nodeId: "b", pinId: "out" })).toBeUndefined();
		expect(index.fromPin({ nodeId: "b", pinId: "missing" })).toBeUndefined();
		expect(index.fromPin({ nodeId: "missing", pinId: "out" })).toBeUndefined();
	});
});

describe("WireIndex.path", () => {
	test("a wire between reroutes lights its chain up to the source and every branch after it", () => {
		const trace = new WireIndex(board()).path("r2-r3");
		expect(sorted(trace?.edges ?? [])).toEqual(
			sorted(["a-r1", "r1-r2", "r2-r3", "r3-c"]),
		);
		expect(sorted(trace?.reroutes ?? [])).toEqual(["r1", "r2", "r3"]);
		expect(ends(trace)).toEqual(["a.out", "c.in"]);
		expect(trace?.segment).toBe("r2-r3");
		expect(trace?.origin).toBeUndefined();
	});

	test("a wire between two nodes is its own path", () => {
		const index = new WireIndex(board());
		const trace = index.trace({ edgeId: "d-a" });
		expect(trace?.edges).toEqual(["d-a"]);
		expect(ends(trace)).toEqual(["a.in", "d.out"]);
		expect(index.trace({ edgeId: "missing" })).toBeUndefined();
	});

	test("a wire into a dead-end reroute ends there", () => {
		expect(ends(new WireIndex(board()).path("e-r4"))).toEqual([
			"e.out",
			"r4.in",
		]);
	});
});

describe("wireEmphasis", () => {
	test("the clicked wire outranks the rest of its path", () => {
		const focus = new WireIndex(board()).path("r1-r2");
		expect(wireEmphasis({ focus }, "r1-r2")).toBe("selected");
		expect(wireEmphasis({ focus }, "a-r1")).toBe("focus");
		expect(wireEmphasis({ focus }, "r2-b")).toBe("focus");
		expect(wireEmphasis({ focus }, "d-a")).toBe("dim");
	});

	test("anchors compare by pin or by wire", () => {
		expect(
			sameAnchor(
				{ pin: { nodeId: "a", pinId: "out" } },
				{ pin: { nodeId: "a", pinId: "out" } },
			),
		).toBe(true);
		expect(sameAnchor({ edgeId: "d-a" }, { edgeId: "d-a" })).toBe(true);
		expect(
			sameAnchor({ edgeId: "d-a" }, { pin: { nodeId: "d", pinId: "out" } }),
		).toBe(false);
		expect(sameAnchor(undefined, undefined)).toBe(true);
		expect(sameAnchor({ edgeId: "d-a" }, undefined)).toBe(false);
	});

	test("focus outranks hover, and a focus dims every other wire", () => {
		const index = new WireIndex(board());
		const hover = index.fromPin({ nodeId: "d", pinId: "out" });
		const focus = index.fromPin({ nodeId: "e", pinId: "out" });
		expect(wireEmphasis(NO_TRACE, "d-a")).toBeUndefined();
		expect(wireEmphasis({ hover }, "d-a")).toBe("hover");
		expect(wireEmphasis({ hover }, "a-r1")).toBeUndefined();
		expect(wireEmphasis({ hover, focus }, "d-a")).toBe("hover");
		expect(wireEmphasis({ hover, focus }, "e-r4")).toBe("focus");
		expect(wireEmphasis({ hover, focus }, "a-r1")).toBe("dim");
	});

	test("endpoints compare by node and pin", () => {
		expect(
			sameEndpoint(
				{ nodeId: "a", pinId: "out" },
				{ nodeId: "a", pinId: "out" },
			),
		).toBe(true);
		expect(
			sameEndpoint({ nodeId: "a", pinId: "out" }, { nodeId: "a", pinId: "in" }),
		).toBe(false);
		expect(sameEndpoint(undefined, { nodeId: "a", pinId: "in" })).toBe(false);
	});
});
