import { describe, expect, it } from "bun:test";
import { type ILayer, ILayerType } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import { resolveFocusTarget, resolveLayerChain } from "./use-layer-navigation";

function layer(
	id: string,
	parentId?: string,
	type: ILayerType = ILayerType.Collapsed,
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
		pins: {},
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
	);
	const nodes: Record<string, INode> = {
		root_node: node("root_node"),
		nested_node: node("nested_node", "inner"),
		fn_node: node("fn_node", "fn"),
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
});
