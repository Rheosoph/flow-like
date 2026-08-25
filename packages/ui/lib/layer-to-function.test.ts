import { describe, expect, test } from "bun:test";
import { owningModuleId, planLayerToFunction } from "./layer-to-function";
import { type IBoard, type ILayer, ILayerType } from "./schema/flow/board";
import {
	ICommandType,
	type IGenericCommand,
} from "./schema/flow/board/commands/generic-command";
import type { INode } from "./schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "./schema/flow/pin";

function pin(
	id: string,
	name: string,
	pin_type: IPinType,
	data_type: IVariableType,
	extra: Partial<IPin> = {},
): IPin {
	return {
		id,
		name,
		friendly_name: name,
		description: "",
		pin_type,
		data_type,
		value_type: IValueType.Normal,
		connected_to: [],
		depends_on: [],
		index: 0,
		...extra,
	};
}

function node(id: string, pins: IPin[], layer?: string): INode {
	return {
		id,
		name: id,
		friendly_name: id,
		description: "",
		category: "",
		coordinates: [0, 0, 0],
		layer: layer ?? null,
		pins: Object.fromEntries(pins.map((p) => [p.id, p])),
	};
}

const callFunctionTemplate: INode = node("control_call_function", [
	pin("template_fn", "function_layer_id", IPinType.Input, IVariableType.String),
]);

/** Flatten a connect/disconnect command into a comparable tuple. */
function wire(command: IGenericCommand) {
	const { from_node, from_pin, to_node, to_pin } = command as unknown as Record<
		string,
		string
	>;
	return [from_node, from_pin, to_node, to_pin];
}

/**
 * outside -> [layer: inner] -> sink
 *   exec + string cross into the layer, exec crosses back out.
 */
function fixture(): { board: IBoard; layer: ILayer } {
	const outside = node("outside", [
		pin("outside_exec", "exec_out", IPinType.Output, IVariableType.Execution, {
			connected_to: ["layer_exec_in"],
		}),
		pin("outside_str", "value", IPinType.Output, IVariableType.String, {
			connected_to: ["layer_str_in"],
		}),
	]);
	const inner = node(
		"inner",
		[
			pin("inner_exec_in", "exec_in", IPinType.Input, IVariableType.Execution, {
				depends_on: ["layer_exec_in"],
			}),
			pin("inner_str_in", "value", IPinType.Input, IVariableType.String, {
				depends_on: ["layer_str_in"],
			}),
			pin(
				"inner_exec_out",
				"exec_out",
				IPinType.Output,
				IVariableType.Execution,
				{ connected_to: ["layer_exec_out"] },
			),
		],
		"layer",
	);
	const sink = node("sink", [
		pin("sink_exec", "exec_in", IPinType.Input, IVariableType.Execution, {
			depends_on: ["layer_exec_out"],
		}),
	]);

	const layer: ILayer = {
		id: "layer",
		name: "Collapsed",
		type: ILayerType.Collapsed,
		parent_id: null,
		coordinates: [120, 240, 0],
		nodes: {},
		comments: {},
		variables: {},
		pins: Object.fromEntries(
			[
				pin(
					"layer_exec_in",
					"exec_in",
					IPinType.Input,
					IVariableType.Execution,
					{
						index: 0,
						depends_on: ["outside_exec"],
						connected_to: ["inner_exec_in"],
					},
				),
				pin("layer_str_in", "value", IPinType.Input, IVariableType.String, {
					index: 1,
					depends_on: ["outside_str"],
					connected_to: ["inner_str_in"],
				}),
				pin(
					"layer_exec_out",
					"exec_out",
					IPinType.Output,
					IVariableType.Execution,
					{
						index: 0,
						depends_on: ["inner_exec_out"],
						connected_to: ["sink_exec"],
					},
				),
			].map((p) => [p.id, p]),
		),
	};

	const board = {
		id: "board",
		nodes: Object.fromEntries([outside, inner, sink].map((n) => [n.id, n])),
		layers: { layer },
		comments: {},
		variables: {},
		refs: {},
	} as unknown as IBoard;

	return { board, layer };
}

describe("planLayerToFunction", () => {
	test("mirrors the boundary onto a call node and rewires the outside", () => {
		const { board, layer } = fixture();
		const result = planLayerToFunction({
			board,
			layer,
			callFunctionTemplate,
		});

		expect(result.ok).toBe(true);
		if (!result.ok) return;
		const { plan } = result;

		expect(plan.movedConnections).toBe(3);
		expect(plan.renamedPins).toBe(0);
		expect(plan.layer.type).toBe(ILayerType.Function);
		expect(plan.layer.parent_id).toBeNull();
		// The signature survives the conversion — same pins, same ids.
		expect(Object.keys(plan.layer.pins).toSorted()).toEqual([
			"layer_exec_in",
			"layer_exec_out",
			"layer_str_in",
		]);

		const callPins = Object.values(plan.callNode.pins);
		expect(callPins.map((p) => `${p.pin_type}:${p.name}`).toSorted()).toEqual([
			"Input:exec_in",
			"Input:function_layer_id",
			"Input:value",
			"Output:exec_out",
		]);
		// Call node pins are freshly minted, never the layer's own pin ids.
		expect(callPins.some((p) => p.id.startsWith("layer_"))).toBe(false);

		const [add, upsert, ...rewires] = plan.commands;
		expect(add.command_type).toBe(ICommandType.AddNode);
		expect(upsert.command_type).toBe(ICommandType.UpsertLayer);
		expect(rewires.map((c) => c.command_type)).toEqual([
			ICommandType.DisconnectPin,
			ICommandType.DisconnectPin,
			ICommandType.DisconnectPin,
			ICommandType.ConnectPin,
			ICommandType.ConnectPin,
			ICommandType.ConnectPin,
		]);

		const callPinId = (name: string, pinType: IPinType) =>
			callPins.find((p) => p.name === name && p.pin_type === pinType)?.id ?? "";

		const connects = rewires
			.filter((c) => c.command_type === ICommandType.ConnectPin)
			.map(wire);
		expect(connects).toContainEqual([
			"outside",
			"outside_exec",
			plan.callNode.id,
			callPinId("exec_in", IPinType.Input),
		]);
		expect(connects).toContainEqual([
			"outside",
			"outside_str",
			plan.callNode.id,
			callPinId("value", IPinType.Input),
		]);
		expect(connects).toContainEqual([
			plan.callNode.id,
			callPinId("exec_out", IPinType.Output),
			"sink",
			"sink_exec",
		]);

		const disconnects = rewires
			.filter((c) => c.command_type === ICommandType.DisconnectPin)
			.map(wire);
		expect(disconnects).toContainEqual([
			"outside",
			"outside_exec",
			"layer",
			"layer_exec_in",
		]);
		expect(disconnects).toContainEqual([
			"layer",
			"layer_exec_out",
			"sink",
			"sink_exec",
		]);
		// The link between the boundary and the nodes inside stays untouched.
		expect(disconnects.some((c) => c.includes("inner_exec_in"))).toBe(false);
	});

	test("places the call node where the layer was, inside the layer's parent", () => {
		const { board, layer } = fixture();
		const nested: ILayer = { ...layer, parent_id: "parent" };
		const result = planLayerToFunction({
			board: {
				...board,
				layers: { ...board.layers, layer: nested, parent: nested },
			} as IBoard,
			layer: nested,
			callFunctionTemplate,
		});

		expect(result.ok).toBe(true);
		if (!result.ok) return;
		expect(result.plan.callNode.coordinates).toEqual([120, 240, 0]);
		expect(
			(result.plan.commands[0] as unknown as { current_layer?: string })
				.current_layer,
		).toBe("parent");
		// "parent" isn't a Module, so the function has no owning module.
		expect(result.plan.layer.parent_id).toBeNull();
	});

	test("owns up to the nearest enclosing Module, however deep it's nested", () => {
		const { board, layer } = fixture();
		const module: ILayer = {
			...layer,
			id: "module",
			type: ILayerType.Module,
			parent_id: null,
		};
		const wrapper: ILayer = {
			...layer,
			id: "wrapper",
			type: ILayerType.Collapsed,
			parent_id: module.id,
		};
		const nested: ILayer = { ...layer, parent_id: wrapper.id };

		const result = planLayerToFunction({
			board: {
				...board,
				layers: { layer: nested, module, wrapper },
			} as IBoard,
			layer: nested,
			callFunctionTemplate,
		});

		expect(result.ok).toBe(true);
		if (!result.ok) return;
		// The call node still lands where the layer visually was, inside "wrapper".
		expect(
			(result.plan.commands[0] as unknown as { current_layer?: string })
				.current_layer,
		).toBe("wrapper");
		// The function layer itself records the module that owns it.
		expect(result.plan.layer.parent_id).toBe("module");
	});

	test("a root-level layer converts to a global function", () => {
		const { board, layer } = fixture();
		const result = planLayerToFunction({ board, layer, callFunctionTemplate });

		expect(result.ok).toBe(true);
		if (!result.ok) return;
		expect(result.plan.layer.parent_id).toBeNull();
	});

	test("owningModuleId is cycle-guarded", () => {
		const layers: IBoard["layers"] = {
			a: { ...fixture().layer, id: "a", parent_id: "b" },
			b: { ...fixture().layer, id: "b", parent_id: "a" },
		};

		expect(owningModuleId(layers, "a")).toBeNull();
	});

	test("renames colliding boundary names so each parameter mirrors separately", () => {
		const { board, layer } = fixture();
		const collided: ILayer = {
			...layer,
			pins: {
				...layer.pins,
				layer_str_in_2: {
					...layer.pins.layer_str_in,
					id: "layer_str_in_2",
					index: 2,
					depends_on: ["outside_str"],
					connected_to: [],
				},
			},
		};

		const result = planLayerToFunction({
			board: { ...board, layers: { layer: collided } } as IBoard,
			layer: collided,
			callFunctionTemplate,
		});

		expect(result.ok).toBe(true);
		if (!result.ok) return;
		expect(result.plan.renamedPins).toBe(1);
		expect(result.plan.layer.pins.layer_str_in.name).toBe("value");
		expect(result.plan.layer.pins.layer_str_in_2.name).toBe("value_2");
		expect(
			Object.values(result.plan.callNode.pins)
				.filter((p) => p.pin_type === IPinType.Input)
				.map((p) => p.name)
				.toSorted(),
		).toEqual(["exec_in", "function_layer_id", "value", "value_2"]);
	});

	test("keeps a boundary pin named like the call node's own pin apart", () => {
		const { board, layer } = fixture();
		const clashing: ILayer = {
			...layer,
			pins: {
				...layer.pins,
				layer_str_in: {
					...layer.pins.layer_str_in,
					name: "function_layer_id",
				},
			},
		};

		const result = planLayerToFunction({
			board: { ...board, layers: { layer: clashing } } as IBoard,
			layer: clashing,
			callFunctionTemplate,
		});

		expect(result.ok).toBe(true);
		if (!result.ok) return;
		expect(result.plan.layer.pins.layer_str_in.name).toBe(
			"function_layer_id_2",
		);
	});

	test("refuses layers whose entry point would be ambiguous", () => {
		const { board, layer } = fixture();
		const twoEntries: ILayer = {
			...layer,
			pins: {
				...layer.pins,
				second_exec_in: {
					...layer.pins.layer_exec_in,
					id: "second_exec_in",
					index: 2,
				},
			},
		};

		const result = planLayerToFunction({
			board,
			layer: twoEntries,
			callFunctionTemplate,
		});
		expect(result).toEqual({
			ok: false,
			error: { reason: "multiple_exec_inputs", count: 2 },
		});
	});

	test("refuses layers that can never be entered", () => {
		const { board, layer } = fixture();
		const { layer_exec_in, ...withoutEntry } = layer.pins;

		const result = planLayerToFunction({
			board,
			layer: { ...layer, pins: withoutEntry },
			callFunctionTemplate,
		});
		expect(result).toEqual({
			ok: false,
			error: { reason: "missing_exec_input" },
		});

		// A layer with no execution boundary at all is a pure function — allowed.
		const { layer_exec_out, ...dataOnly } = withoutEntry;
		expect(
			planLayerToFunction({
				board,
				layer: { ...layer, pins: dataOnly },
				callFunctionTemplate,
			}).ok,
		).toBe(true);
	});

	test("refuses layers that are already functions and boards without the catalog node", () => {
		const { board, layer } = fixture();
		expect(
			planLayerToFunction({
				board,
				layer: { ...layer, type: ILayerType.Function },
				callFunctionTemplate,
			}),
		).toEqual({ ok: false, error: { reason: "already_function" } });

		expect(
			planLayerToFunction({
				board,
				layer,
				callFunctionTemplate: undefined,
			}),
		).toEqual({ ok: false, error: { reason: "missing_catalog_node" } });
	});
});
