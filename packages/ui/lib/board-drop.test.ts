import { describe, expect, test } from "bun:test";
import {
	boardDropPayload,
	planBoardDrop,
	withFunctionPins,
} from "./board-drop";
import type { ILayer } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";
import { type IPin, IPinType } from "./schema/flow/pin";
import type { IVariable } from "./schema/flow/variable";
import { convertJsonToUint8Array } from "./uint8";

function pin(
	id: string,
	name: string,
	pinType: IPinType,
	index = 1,
	dataType = "String",
): IPin {
	return {
		id,
		name,
		friendly_name: name,
		pin_type: pinType,
		data_type: dataType,
		value_type: "Normal",
		depends_on: [],
		connected_to: [],
		index,
		description: "",
	} as unknown as IPin;
}

function node(name: string, pins: IPin[]): INode {
	return {
		id: `${name}-template`,
		name,
		friendly_name: name,
		description: "",
		category: "Test",
		coordinates: [40, 80, 0],
		pins: Object.fromEntries(pins.map((entry) => [entry.id, entry])),
	} as unknown as INode;
}

const CATALOG = [
	node("control_call_function", [
		pin("exec_in", "exec_in", IPinType.Input, 0, "Execution"),
		pin("layer", "function_layer_id", IPinType.Input),
		pin("exec_out", "exec_out", IPinType.Output, 0, "Execution"),
	]),
	node("variable_get", [
		pin("get_ref", "var_ref", IPinType.Input),
		pin("get_value", "value", IPinType.Output),
	]),
	node("variable_set", [
		pin("set_ref", "var_ref", IPinType.Input),
		pin("set_value", "value", IPinType.Input),
	]),
	node("path_from_upload_dir", [pin("u1", "path", IPinType.Output)]),
	node("path_from_user_dir", [
		pin("s1", "path", IPinType.Output),
		pin("s2", "node_scope", IPinType.Input),
	]),
	node("child", [
		pin("c1", "path", IPinType.Output),
		pin("c2", "parent_path", IPinType.Input),
		pin("c3", "child_name", IPinType.Input),
	]),
];

const VARIABLE = {
	id: "var-1",
	name: "Counter",
	data_type: "Integer",
	value_type: "Normal",
	editable: true,
	exposed: false,
	secret: false,
} as unknown as IVariable;

function pinNamed(entry: INode | undefined, name: string) {
	return Object.values(entry?.pins ?? {}).find((p) => p.name === name);
}

describe("boardDropPayload", () => {
	test("reads each shell drag the way the drop handler does", () => {
		expect(
			boardDropPayload({ type: "function-layer", layerId: "layer-1" }),
		).toEqual({ type: "function-layer", layerId: "layer-1" });
		expect(
			boardDropPayload({ type: "storage-path", scope: "user", path: "a.txt" }),
		).toEqual({ type: "storage-path", scope: "user", path: "a.txt" });
		expect(boardDropPayload(VARIABLE)).toEqual({
			type: "variable",
			variable: VARIABLE,
		});
	});

	test("drags the board cannot take preview nothing", () => {
		expect(boardDropPayload(undefined)).toBeUndefined();
		expect(boardDropPayload("layer-1")).toBeUndefined();
		expect(boardDropPayload({ type: "function-layer" })).toBeUndefined();
		expect(
			boardDropPayload({ type: "storage-path", scope: "board", path: "a" }),
		).toBeUndefined();
		expect(boardDropPayload({ id: "folder", name: "Folder" })).toBeUndefined();
	});
});

describe("planBoardDrop", () => {
	test("a function becomes one call node bound to its layer", () => {
		const nodes = planBoardDrop(
			{ type: "function-layer", layerId: "layer-1" },
			CATALOG,
		);
		expect(nodes).toHaveLength(1);
		expect(nodes?.[0]?.coordinates).toEqual([0, 0, 0]);
		expect(pinNamed(nodes?.[0], "function_layer_id")?.default_value).toEqual(
			convertJsonToUint8Array("layer-1"),
		);
	});

	test("a variable shows as a getter until the user picks", () => {
		const payload = { type: "variable", variable: VARIABLE } as const;
		const getter = planBoardDrop(payload, CATALOG)?.[0];
		const setter = planBoardDrop(payload, CATALOG, "set")?.[0];
		expect(getter?.name).toBe("variable_get");
		expect(setter?.name).toBe("variable_set");
		expect(pinNamed(setter, "var_ref")?.default_value).toEqual(
			convertJsonToUint8Array("var-1"),
		);
	});

	test("binding a pin leaves the shared catalog template untouched", () => {
		planBoardDrop({ type: "variable", variable: VARIABLE }, CATALOG);
		expect(pinNamed(CATALOG[1], "var_ref")?.default_value).toBeUndefined();
		expect(CATALOG[1]?.coordinates).toEqual([40, 80, 0]);
	});

	test("a stored file plans the directory and child pair from the drop point", () => {
		const nodes = planBoardDrop(
			{ type: "storage-path", scope: "app", path: "docs/a.pdf" },
			CATALOG,
		);
		expect(nodes?.map((entry) => entry.name)).toEqual([
			"path_from_upload_dir",
			"child",
		]);
		expect(nodes?.[0]?.coordinates?.slice(0, 2)).toEqual([0, 0]);
		expect(nodes?.[1]?.coordinates?.[0]).toBeGreaterThan(0);
	});

	test("a catalog without the template plans nothing", () => {
		expect(
			planBoardDrop({ type: "function-layer", layerId: "layer-1" }, []),
		).toBeUndefined();
		expect(
			planBoardDrop({ type: "variable", variable: VARIABLE }, undefined),
		).toBeUndefined();
		expect(
			planBoardDrop({ type: "storage-path", scope: "app", path: "a" }, []),
		).toBeUndefined();
	});
});

describe("withFunctionPins", () => {
	const call = planBoardDrop(
		{ type: "function-layer", layerId: "layer-1" },
		CATALOG,
	)?.[0] as INode;
	const layer = {
		id: "layer-1",
		name: "Fetch",
		pins: {
			b: pin("b", "second", IPinType.Input, 2),
			a: pin("a", "first", IPinType.Input, 1),
			r: { ...pin("r", "result", IPinType.Output, 1), connected_to: ["x"] },
		},
	} as unknown as ILayer;

	test("names the call after the function and mirrors its pins in order", () => {
		const completed = withFunctionPins(call, layer);
		expect(completed.friendly_name).toBe("Call Fetch");
		const mirrored = Object.values(completed.pins).filter((entry) =>
			entry.id.startsWith(`${call.id}:`),
		);
		expect(
			mirrored.map((entry) => [entry.name, entry.pin_type, entry.index]),
		).toEqual([
			["first", IPinType.Input, 2],
			["second", IPinType.Input, 3],
			["result", IPinType.Output, 1],
		]);
		expect(mirrored.every((entry) => entry.connected_to.length === 0)).toBe(
			true,
		);
	});

	test("an unknown layer leaves the call node as the catalog has it", () => {
		expect(withFunctionPins(call, undefined)).toBe(call);
	});
});
