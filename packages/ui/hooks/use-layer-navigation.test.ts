import { describe, expect, it } from "bun:test";
import { type ILayer, ILayerType } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import type { IPin } from "../lib/schema/flow/pin";
import {
	type LayerVisit,
	dropVisitsTo,
	focusSentinelId,
	isFocusRendered,
	parentPath,
	recordVisit,
	resolveExit,
	resolveFocusTarget,
	resolveLayerChain,
} from "./use-layer-navigation";

function layer(
	id: string,
	parentId?: string,
	type: ILayerType = ILayerType.Collapsed,
	pins: Record<string, IPin> = {},
): ILayer {
	return {
		id,
		parent_id: parentId ?? null,
		name: id,
		type,
		nodes: {},
		variables: {},
		comments: {},
		coordinates: [0, 0, 0],
		pins,
	} as unknown as ILayer;
}

function node(id: string, layerId?: string): INode {
	return { id, name: id, layer: layerId ?? null } as unknown as INode;
}

function layerMap(...entries: ILayer[]): Record<string, ILayer> {
	return Object.fromEntries(entries.map((entry) => [entry.id, entry]));
}

describe("resolveLayerChain", () => {
	it("returns an empty chain for root", () => {
		expect(resolveLayerChain(layerMap(), undefined)).toEqual([]);
		expect(resolveLayerChain(layerMap(), null)).toEqual([]);
		expect(resolveLayerChain(layerMap(layer("a")), "")).toEqual([]);
	});

	it("orders ancestors outermost first", () => {
		const layers = layerMap(layer("a"), layer("b", "a"), layer("c", "b"));
		expect(resolveLayerChain(layers, "c")).toEqual(["a", "b", "c"]);
	});

	it("treats an empty parent_id as root", () => {
		const layers = layerMap(layer("a", ""));
		expect(resolveLayerChain(layers, "a")).toEqual(["a"]);
	});

	it("stops at a parent that no longer exists", () => {
		const layers = layerMap(layer("b", "missing"), layer("c", "b"));
		expect(resolveLayerChain(layers, "c")).toEqual(["b", "c"]);
	});

	it("terminates on a cyclic parent chain", () => {
		const layers = layerMap(layer("a", "b"), layer("b", "a"));
		expect(resolveLayerChain(layers, "a")).toEqual(["b", "a"]);
	});

	it("returns an empty chain for an unknown layer", () => {
		expect(resolveLayerChain(layerMap(layer("a")), "nope")).toEqual([]);
	});
});

describe("resolveFocusTarget", () => {
	const layers = layerMap(
		layer("outer"),
		layer("inner", "outer"),
		layer("fn", "outer", ILayerType.Function),
		layer("mod", undefined, ILayerType.Module),
		layer("nested_mod", "mod", ILayerType.Module),
		layer("mod_fn", "mod", ILayerType.Function),
	);
	const nodes: Record<string, INode> = {
		root_node: node("root_node"),
		nested_node: node("nested_node", "inner"),
		fn_node: node("fn_node", "fn"),
		mod_node: node("mod_node", "mod"),
	};

	it("focuses a root node without opening a layer", () => {
		expect(resolveFocusTarget(nodes, layers, "root_node")).toEqual({
			chain: [],
			renderTargetId: "root_node",
		});
	});

	it("opens the full layer chain of a nested node", () => {
		expect(resolveFocusTarget(nodes, layers, "nested_node")).toEqual({
			chain: ["outer", "inner"],
			renderTargetId: "nested_node",
		});
	});

	it("opens the function body of a node inside a function", () => {
		expect(resolveFocusTarget(nodes, layers, "fn_node")).toEqual({
			chain: ["outer", "fn"],
			renderTargetId: "fn_node",
		});
	});

	it("reveals a layer inside its parent, since it is drawn there", () => {
		expect(resolveFocusTarget(nodes, layers, "inner")).toEqual({
			chain: ["outer"],
			renderTargetId: "inner",
		});
	});

	it("enters a function, which has no node on its parent canvas", () => {
		expect(resolveFocusTarget(nodes, layers, "fn")).toEqual({
			chain: ["outer", "fn"],
			renderTargetId: undefined,
		});
	});

	it("reveals a root-level layer at the board root", () => {
		expect(resolveFocusTarget(nodes, layers, "outer")).toEqual({
			chain: [],
			renderTargetId: "outer",
		});
	});

	it("returns undefined for an id that is neither node nor layer", () => {
		expect(resolveFocusTarget(nodes, layers, "deleted")).toBeUndefined();
	});

	it("opens a module, which is a file rather than a node on any canvas", () => {
		expect(resolveFocusTarget(nodes, layers, "mod")).toEqual({
			chain: ["mod"],
			renderTargetId: undefined,
		});
	});

	it("opens a nested module through its module ancestors", () => {
		expect(resolveFocusTarget(nodes, layers, "nested_mod")).toEqual({
			chain: ["mod", "nested_mod"],
			renderTargetId: undefined,
		});
	});

	it("opens a module-local function inside its module", () => {
		expect(resolveFocusTarget(nodes, layers, "mod_fn")).toEqual({
			chain: ["mod", "mod_fn"],
			renderTargetId: undefined,
		});
	});

	it("centres a node that lives in a module", () => {
		expect(resolveFocusTarget(nodes, layers, "mod_node")).toEqual({
			chain: ["mod"],
			renderTargetId: "mod_node",
		});
	});
});

describe("focusSentinelId", () => {
	const pin = { id: "pin" } as unknown as IPin;
	const layers = layerMap(
		layer("bridged", undefined, ILayerType.Collapsed, { pin }),
		layer("pinless", undefined, ILayerType.Collapsed),
		layer("mod", undefined, ILayerType.Module),
	);

	it("waits for the target node when there is one", () => {
		expect(focusSentinelId(layers, "bridged", "node_a")).toBe("node_a");
	});

	it("waits for the boundary of a layer that draws one", () => {
		expect(focusSentinelId(layers, "bridged", undefined)).toBe("bridged-input");
	});

	it("has no sentinel for a pin-less layer, which never draws a boundary", () => {
		expect(focusSentinelId(layers, "pinless", undefined)).toBeUndefined();
		expect(focusSentinelId(layers, "mod", undefined)).toBeUndefined();
	});

	it("has no sentinel for the board root", () => {
		expect(focusSentinelId(layers, undefined, undefined)).toBeUndefined();
	});

	it("has no sentinel for a layer that is gone", () => {
		expect(focusSentinelId(layers, "deleted", undefined)).toBeUndefined();
	});
});

describe("isFocusRendered", () => {
	const ready = (
		renderedIds: string[],
		sentinelId: string | undefined,
		baseline: string[],
		switchesLayer = true,
	) =>
		isFocusRendered({
			renderedIds,
			sentinelId,
			baselineIds: new Set(baseline),
			switchesLayer,
		});

	it("waits for the sentinel to appear", () => {
		expect(ready(["a"], "b-input", ["a"])).toBe(false);
		expect(ready(["a", "b-input"], "b-input", ["a"])).toBe(true);
	});

	it("is ready immediately when the view does not change", () => {
		expect(ready(["a"], undefined, ["a"], false)).toBe(true);
	});

	it("waits for the canvas to move off what the focus started on", () => {
		expect(ready(["a", "b"], undefined, ["a", "b"])).toBe(false);
		expect(ready(["c"], undefined, ["a", "b"])).toBe(true);
		expect(ready(["a", "c"], undefined, ["a", "b"])).toBe(true);
	});

	it("treats an emptied canvas as arrived", () => {
		expect(ready([], undefined, ["a"])).toBe(true);
	});

	it("does not stall when an empty view follows an empty one", () => {
		expect(ready([], undefined, [])).toBe(true);
	});
});

describe("parentPath", () => {
	it("is undefined for a top-level layer", () => {
		expect(parentPath("a")).toBeUndefined();
	});

	it("drops the last segment", () => {
		expect(parentPath("a/b")).toBe("a");
		expect(parentPath("a/b/c")).toBe("a/b");
	});
});

describe("layer trail", () => {
	/** Walks the same sequence of pushes and pops the hook performs. */
	function walk(steps: (string | "up")[]): {
		path: string | undefined;
		trail: LayerVisit[];
	} {
		let path: string | undefined;
		let trail: LayerVisit[] = [];

		for (const step of steps) {
			if (step === "up") {
				if (!path) continue;
				const exit = resolveExit(trail, path);
				path = exit.path;
				trail = exit.trail;
				continue;
			}
			trail = recordVisit(trail, { from: path, to: step });
			path = step;
		}

		return { path, trail };
	}

	it("leaves a nested layer for its parent", () => {
		expect(walk(["a", "a/b", "a/b/c", "up"]).path).toBe("a/b");
	});

	it("leaves a top-level layer for the board root", () => {
		expect(walk(["a", "up"]).path).toBeUndefined();
	});

	it("returns to the function a nested function was opened from", () => {
		// Both functions hang off the root, so their paths are single segments.
		const { path, trail } = walk(["outer_fn", "inner_fn", "up"]);
		expect(path).toBe("outer_fn");
		expect(trail).toHaveLength(1);
	});

	it("unwinds a whole chain of functions one step at a time", () => {
		expect(walk(["a", "fn_one", "fn_two", "up"]).path).toBe("fn_one");
		expect(walk(["a", "fn_one", "fn_two", "up", "up"]).path).toBe("a");
		expect(
			walk(["a", "fn_one", "fn_two", "up", "up", "up"]).path,
		).toBeUndefined();
	});

	it("keeps the layer a function was called from, not the function's own parent", () => {
		expect(walk(["a", "a/b", "fn", "up"]).path).toBe("a/b");
	});

	it("unwinds one step at a time when a function is entered twice", () => {
		expect(walk(["fn_a", "fn_b", "fn_a", "up"]).path).toBe("fn_b");
		expect(walk(["fn_a", "fn_b", "fn_a", "up", "up"]).path).toBe("fn_a");
		expect(
			walk(["fn_a", "fn_b", "fn_a", "up", "up", "up"]).path,
		).toBeUndefined();
	});

	it("ignores re-opening the layer already on screen", () => {
		expect(walk(["fn", "fn", "up"]).path).toBeUndefined();
	});

	it("falls back to the parent chain when the trail does not lead here", () => {
		// A breadcrumb or goto moved the user without walking in.
		const trail: LayerVisit[] = [{ from: undefined, to: "fn" }];
		const exit = resolveExit(trail, "a/b/c");
		expect(exit.path).toBe("a/b");
		expect(exit.trail).toEqual([]);
	});

	it("forgets the steps into a layer that was jumped to", () => {
		const trail: LayerVisit[] = [
			{ from: undefined, to: "a" },
			{ from: "a", to: "fn" },
		];
		const jumped = dropVisitsTo(trail, "fn");
		expect(jumped).toEqual([{ from: undefined, to: "a" }]);
		expect(resolveExit(jumped, "fn").path).toBeUndefined();
	});

	it("keeps an unrelated trail intact on a jump", () => {
		const trail: LayerVisit[] = [{ from: undefined, to: "a" }];
		expect(dropVisitsTo(trail, "other")).toEqual(trail);
	});

	it("bounds the trail", () => {
		let trail: LayerVisit[] = [];
		for (let index = 0; index < 200; index++) {
			trail = recordVisit(trail, { from: `l${index}`, to: `l${index + 1}` });
		}
		expect(trail).toHaveLength(64);
		expect(trail[trail.length - 1].to).toBe("l200");
	});
});
