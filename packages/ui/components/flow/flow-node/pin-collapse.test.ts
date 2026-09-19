import { describe, expect, test } from "bun:test";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import type { IPinAction } from "../flow-node";
import {
	isPinCollapsible,
	isPinConnected,
	layoutCollapsiblePins,
} from "./pin-collapse";

function pin(
	id: string,
	index: number,
	pinType: IPinType,
	overrides: Partial<IPin> = {},
): IPin {
	return {
		id,
		index,
		name: id,
		friendly_name: id,
		description: "",
		pin_type: pinType,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		connected_to: [],
		depends_on: [],
		...overrides,
	};
}

function action(target: IPin): IPinAction {
	return { action: "create", pin: target, onAction: async () => {} };
}

const execIn = pin("exec_in", 1, IPinType.Input, {
	data_type: IVariableType.Execution,
});
const wiredIn = pin("wired_in", 2, IPinType.Input, { depends_on: ["src"] });
const looseIn = pin("loose_in", 3, IPinType.Input);
const execOut = pin("exec_out", 1, IPinType.Output, {
	data_type: IVariableType.Execution,
});
const looseOutA = pin("loose_a", 2, IPinType.Output);
const wiredOut = pin("wired_out", 3, IPinType.Output, {
	connected_to: ["dst"],
});
const looseOutB = pin("loose_b", 4, IPinType.Output);

describe("pin connection predicates", () => {
	test("either wiring direction counts as connected", () => {
		expect(isPinConnected(wiredIn)).toBe(true);
		expect(isPinConnected(wiredOut)).toBe(true);
		expect(isPinConnected(looseIn)).toBe(false);
	});

	test("execution pins are never collapsible", () => {
		expect(isPinCollapsible(execIn)).toBe(false);
		expect(isPinCollapsible(execOut)).toBe(false);
		expect(isPinCollapsible(looseOutA)).toBe(true);
		expect(isPinCollapsible(wiredOut)).toBe(false);
	});
});

describe("layoutCollapsiblePins", () => {
	const inputs = [execIn, wiredIn, looseIn];
	const outputs = [execOut, looseOutA, wiredOut, looseOutB, action(looseOutB)];

	test("expanded keeps rows untouched and only reports what could be hidden", () => {
		const layout = layoutCollapsiblePins(inputs, outputs, false);
		expect(layout.inputs).toBe(inputs);
		expect(layout.outputs).toBe(outputs);
		expect(layout.slots).toEqual({});
		expect(layout.hiddenCount).toBe(0);
		expect(layout.collapsibleCount).toBe(3);
	});

	test("collapsed drops unconnected data pins and add-pin actions, re-packing slots", () => {
		const layout = layoutCollapsiblePins(inputs, outputs, true);
		expect(layout.inputs.map((row) => (row as IPin).id)).toEqual([
			"exec_in",
			"wired_in",
		]);
		expect(layout.outputs.map((row) => (row as IPin).id)).toEqual([
			"exec_out",
			"wired_out",
		]);
		expect(layout.slots).toEqual({
			exec_in: 1,
			wired_in: 2,
			exec_out: 1,
			wired_out: 2,
		});
		expect(layout.hiddenCount).toBe(3);
		expect(layout.collapsibleCount).toBe(3);
	});

	test("collapsing never mutates the source pins or their indices", () => {
		const before = JSON.stringify([inputs, outputs]);
		layoutCollapsiblePins(inputs, outputs, true);
		expect(JSON.stringify([inputs, outputs])).toBe(before);
		expect(wiredOut.index).toBe(3);
	});

	test("a fully wired node has nothing to collapse", () => {
		const layout = layoutCollapsiblePins(
			[execIn, wiredIn],
			[execOut, wiredOut],
			true,
		);
		expect(layout.hiddenCount).toBe(0);
		expect(layout.collapsibleCount).toBe(0);
		expect(layout.inputs).toHaveLength(2);
	});
});
