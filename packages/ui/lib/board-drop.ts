import type { ILayer } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";
import { IPinType } from "./schema/flow/pin";
import type { IVariable } from "./schema/flow/variable";
import { buildStoragePathNodes } from "./storage-path-nodes";
import type { IStorageScope } from "./storage-tree";
import { convertJsonToUint8Array } from "./uint8";

/** What the shell can drop onto a board. */
export type BoardDropPayload =
	| { type: "function-layer"; layerId: string }
	| { type: "storage-path"; scope: IStorageScope; path: string }
	| { type: "variable"; variable: IVariable };

const isStorageScope = (scope: unknown): scope is IStorageScope =>
	scope === "app" || scope === "user";

function typedPayload(
	value: Record<string, unknown>,
): BoardDropPayload | undefined {
	if (value.type === "function-layer" && typeof value.layerId === "string")
		return { type: "function-layer", layerId: value.layerId };
	if (
		value.type === "storage-path" &&
		isStorageScope(value.scope) &&
		typeof value.path === "string"
	)
		return { type: "storage-path", scope: value.scope, path: value.path };
	return undefined;
}

/** Reads a dnd-kit drag's data the way the board's drop handler does. */
export function boardDropPayload(data: unknown): BoardDropPayload | undefined {
	if (!data || typeof data !== "object") return undefined;
	const value = data as Record<string, unknown>;
	// Variable tokens drag the variable itself, which carries no drag type.
	if (typeof value.id === "string" && typeof value.data_type === "string")
		return { type: "variable", variable: data as IVariable };
	return typedPayload(value);
}

/**
 * A copy of `template` with one pin's default set.
 *
 * The catalog is a shared query result: writing a default straight onto it left the last
 * dragged variable's id baked into the palette's own `variable_get` for the rest of the
 * session.
 */
export function withPinDefault(
	template: INode | undefined,
	pinName: string,
	value: unknown,
): INode | undefined {
	if (!template) return undefined;
	const target = Object.values(template.pins).find(
		(pin) => pin.name === pinName,
	);
	if (!target) return undefined;
	return {
		...template,
		pins: {
			...template.pins,
			[target.id]: {
				...target,
				default_value: convertJsonToUint8Array(value),
			},
		},
	};
}

/**
 * The nodes a drop creates. Coordinates are relative to the drop point, which becomes the
 * first node's top-left corner. Until the user picks one, a variable is shown as a getter.
 */
export function planBoardDrop(
	payload: BoardDropPayload,
	catalog: readonly INode[] | undefined,
	operation: "get" | "set" = "get",
): INode[] | undefined {
	const template = (name: string) =>
		catalog?.find((node) => node.name === name);
	const single = (node: INode | undefined) =>
		node && [{ ...node, coordinates: [0, 0, 0] as [number, number, number] }];
	switch (payload.type) {
		case "function-layer":
			return single(
				withPinDefault(
					template("control_call_function"),
					"function_layer_id",
					payload.layerId,
				),
			);
		case "variable":
			return single(
				withPinDefault(
					template(`variable_${operation}`),
					"var_ref",
					payload.variable.id,
				),
			);
		case "storage-path":
			return buildStoragePathNodes({
				catalog,
				scope: payload.scope,
				path: payload.path,
			})?.nodes;
	}
}

/**
 * The call node as the backend completes it: it takes the function's name and mirrors its
 * inputs after the hidden layer reference and its outputs from the first row.
 */
export function withFunctionPins(
	node: INode,
	layer: ILayer | undefined,
): INode {
	if (!layer) return node;
	const mirror = (type: IPinType, first: number) =>
		Object.values(layer.pins ?? {})
			.filter((pin) => pin.pin_type === type)
			.sort((a, b) => a.index - b.index)
			.map((pin, index) => ({
				...pin,
				id: `${node.id}:${pin.id}`,
				index: index + first,
				connected_to: [],
				depends_on: [],
			}));
	const pins = [
		...Object.values(node.pins),
		...mirror(IPinType.Input, 2),
		...mirror(IPinType.Output, 1),
	];
	return {
		...node,
		friendly_name: `Call ${layer.name}`,
		pins: Object.fromEntries(pins.map((pin) => [pin.id, pin])),
	};
}
