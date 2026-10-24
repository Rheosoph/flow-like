import { describe, expect, it } from "bun:test";
import { migratedBoundaryPins, placeholderPin } from "./layer-boundary-pins";
import type { PlaceholderPinDef } from "./schema/flow/copilot";
import { IVariableType } from "./schema/flow/node";
import {
	type IPin,
	type IPinOptions,
	IPinType,
	IValueType,
} from "./schema/flow/pin";

function pin(
	id: string,
	name: string,
	pinType: IPinType,
	dataType: IVariableType,
	index: number,
	options: IPinOptions | null = null,
): IPin {
	return {
		id,
		name,
		friendly_name: name,
		description: `${name} description`,
		pin_type: pinType,
		data_type: dataType,
		value_type: IValueType.Normal,
		index,
		connected_to: [`${id}-target`],
		depends_on: [`${id}-source`],
		default_value: null,
		schema: null,
		options,
	};
}

function def(
	name: string,
	pinType: "Input" | "Output",
	dataType: PlaceholderPinDef["data_type"],
	extra: Partial<PlaceholderPinDef> = {},
): PlaceholderPinDef {
	return {
		name,
		friendly_name: name,
		pin_type: pinType,
		data_type: dataType,
		...extra,
	};
}

function functionPins(): Record<string, IPin> {
	const pins = [
		pin("exec_in", "exec_in", IPinType.Input, IVariableType.Execution, 0),
		pin("exec_out", "exec_out", IPinType.Output, IVariableType.Execution, 1),
		pin("p_name", "name", IPinType.Input, IVariableType.String, 2),
		pin("p_count", "count", IPinType.Input, IVariableType.Integer, 3, {
			enforce_schema: true,
			sensitive: true,
		}),
		pin("r_result", "result", IPinType.Output, IVariableType.String, 4),
	];
	return Object.fromEntries(pins.map((entry) => [entry.id, entry]));
}

function byName(
	pins: Record<string, IPin>,
	name: string,
	pinType: IPinType,
): IPin | undefined {
	return Object.values(pins).find(
		(entry) => entry.name === name && entry.pin_type === pinType,
	);
}

describe("migratedBoundaryPins", () => {
	it("keeps execution pins untouched", () => {
		const current = functionPins();
		const pins = migratedBoundaryPins(current, []);

		expect(Object.keys(pins).sort()).toEqual(["exec_in", "exec_out"]);
		expect(pins.exec_in).toEqual(current.exec_in);
		expect(pins.exec_out).toEqual(current.exec_out);
	});

	it("keeps id and wiring of a surviving pin and applies the new type", () => {
		const pins = migratedBoundaryPins(functionPins(), [
			def("name", "Input", "Struct", {
				value_type: "Array",
				schema: '{"type":"object"}',
			}),
		]);

		const kept = pins.p_name;
		expect(kept).toBeDefined();
		expect(kept.data_type).toBe(IVariableType.Struct);
		expect(kept.value_type).toBe(IValueType.Array);
		expect(kept.schema).toBe('{"type":"object"}');
		expect(kept.connected_to).toEqual(["p_name-target"]);
		expect(kept.depends_on).toEqual(["p_name-source"]);
		expect(kept.description).toBe("name description");
	});

	it("matches surviving pins by name and direction only", () => {
		const pins = migratedBoundaryPins(functionPins(), [
			def("result", "Input", "String"),
		]);

		const input = byName(pins, "result", IPinType.Input);
		expect(input?.id).not.toBe("r_result");
		expect(input?.connected_to).toEqual([]);
		expect(pins.r_result).toBeUndefined();
	});

	it("turns a renamed pin into a new unwired pin", () => {
		const pins = migratedBoundaryPins(functionPins(), [
			def("full_name", "Input", "String", { description: "Who" }),
		]);

		expect(pins.p_name).toBeUndefined();
		const renamed = byName(pins, "full_name", IPinType.Input);
		expect(renamed).toBeDefined();
		expect(renamed?.id).not.toBe("p_name");
		expect(renamed?.connected_to).toEqual([]);
		expect(renamed?.depends_on).toEqual([]);
		expect(renamed?.description).toBe("Who");
		expect(renamed?.default_value).toBeNull();
		expect(renamed?.value_type).toBe(IValueType.Normal);
		expect(renamed?.options).toBeNull();
	});

	it("indexes data pins after the execution pins in written order", () => {
		const pins = migratedBoundaryPins(functionPins(), [
			def("result", "Output", "String"),
			def("count", "Input", "Integer"),
			def("extra", "Input", "Boolean"),
		]);

		expect(pins.exec_in.index).toBe(0);
		expect(pins.exec_out.index).toBe(1);
		expect(pins.r_result.index).toBe(2);
		expect(pins.p_count.index).toBe(3);
		expect(byName(pins, "extra", IPinType.Input)?.index).toBe(4);
		expect(Object.keys(pins)).toHaveLength(5);
	});

	it("claims each surviving pin at most once", () => {
		const pins = migratedBoundaryPins(functionPins(), [
			def("name", "Input", "String"),
			def("name", "Input", "String"),
		]);

		const names = Object.values(pins).filter((entry) => entry.name === "name");
		expect(names).toHaveLength(2);
		expect(names.filter((entry) => entry.id === "p_name")).toHaveLength(1);
	});

	it("sets and clears enforce_schema while keeping other options", () => {
		const pins = migratedBoundaryPins(functionPins(), [
			def("name", "Input", "Struct", { enforce_schema: true }),
			def("count", "Input", "Integer"),
			def("result", "Output", "String"),
			def("fresh", "Output", "Struct", { enforce_schema: true }),
		]);

		expect(pins.p_name.options).toEqual({ enforce_schema: true });
		expect(pins.p_count.options).toEqual({ sensitive: true });
		expect(pins.r_result.options).toBeNull();
		expect(byName(pins, "fresh", IPinType.Output)?.options).toEqual({
			enforce_schema: true,
		});
	});
});

describe("placeholderPin", () => {
	it("reads anything but Output as an Input, like the Rust applier", () => {
		const pin = placeholderPin(
			def("odd", "Sideways" as PlaceholderPinDef["pin_type"], "String"),
			7,
		);

		expect(pin.pin_type).toBe(IPinType.Input);
		expect(pin.index).toBe(7);
		expect(pin.schema).toBeNull();
	});
});
