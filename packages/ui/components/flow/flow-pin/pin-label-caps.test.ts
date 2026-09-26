import { describe, expect, test } from "bun:test";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import { convertJsonToUint8Array } from "../../../lib/uint8";
import type { IPinAction } from "../flow-node";
import {
	PIN_LABEL_CAP_SOLO,
	PIN_ROW_LABEL_BUDGET,
	estimateTextPx,
	pinLabelNeed,
	planPinLabelCaps,
	splitRowCaps,
} from "./pin-label-caps";

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

const LONG = "Exponential Backoff With Full Jitter";

describe("splitRowCaps", () => {
	test("both long sides share the budget evenly", () => {
		expect(splitRowCaps(200, 180)).toEqual([
			PIN_ROW_LABEL_BUDGET / 2,
			PIN_ROW_LABEL_BUDGET / 2,
		]);
	});

	test("short side keeps its need, long side gets the rest", () => {
		expect(splitRowCaps(30, 200)).toEqual([30, PIN_ROW_LABEL_BUDGET - 30]);
		expect(splitRowCaps(200, 30)).toEqual([PIN_ROW_LABEL_BUDGET - 30, 30]);
	});

	test("one-sided rows get the solo cap", () => {
		expect(splitRowCaps(0, 200)).toEqual([
			PIN_LABEL_CAP_SOLO,
			PIN_LABEL_CAP_SOLO,
		]);
	});
});

describe("pinLabelNeed", () => {
	test("exec pins never need label space", () => {
		expect(
			pinLabelNeed(
				pin("exec_out", 1, IPinType.Output, {
					data_type: IVariableType.Execution,
				}),
			),
		).toBe(0);
	});

	test("dynamic pins reserve the delete button", () => {
		const base = pin("value", 2, IPinType.Input);
		expect(pinLabelNeed({ ...base, dynamic: true })).toBe(
			pinLabelNeed(base) + 16,
		);
	});

	test("enum need follows the decoded option text", () => {
		const options = { valid_values: ["Fixed", LONG] };
		const short = pin("mode", 2, IPinType.Input, {
			options,
			default_value: convertJsonToUint8Array("Fixed"),
		});
		const long = { ...short, default_value: convertJsonToUint8Array(LONG) };
		expect(pinLabelNeed(long)).toBeGreaterThan(pinLabelNeed(short));
		expect(pinLabelNeed(long) - pinLabelNeed(short)).toBe(
			estimateTextPx(LONG) - estimateTextPx("Fixed"),
		);
	});
});

describe("planPinLabelCaps", () => {
	test("short input opposite a long output renders in full", () => {
		const input = pin("Mode", 2, IPinType.Input, { depends_on: ["src"] });
		const output = pin(LONG, 2, IPinType.Output);
		const caps = planPinLabelCaps([input], [output]);
		const need = pinLabelNeed(input);
		expect(need).toBeLessThanOrEqual(PIN_ROW_LABEL_BUDGET / 2);
		expect(caps[input.id]).toBe(need);
		expect(caps[output.id]).toBe(PIN_ROW_LABEL_BUDGET - need);
	});

	test("both long sides split evenly", () => {
		const input = pin(`${LONG} In`, 2, IPinType.Input, { depends_on: ["s"] });
		const output = pin(`${LONG} Out`, 2, IPinType.Output);
		const caps = planPinLabelCaps([input], [output]);
		expect(caps[input.id]).toBe(PIN_ROW_LABEL_BUDGET / 2);
		expect(caps[output.id]).toBe(PIN_ROW_LABEL_BUDGET / 2);
	});

	test("one-sided rows and rows opposite exec pins are solo", () => {
		const alone = pin("alone", 3, IPinType.Input);
		const data = pin("data", 1, IPinType.Input);
		const execOut = pin("exec_out", 1, IPinType.Output, {
			data_type: IVariableType.Execution,
		});
		const caps = planPinLabelCaps([alone, data], [execOut]);
		expect(caps[alone.id]).toBe(PIN_LABEL_CAP_SOLO);
		expect(caps[data.id]).toBe(PIN_LABEL_CAP_SOLO);
		expect(caps[execOut.id]).toBeUndefined();
	});

	test("pin action rows are ignored", () => {
		const input = pin("in", 2, IPinType.Input);
		const output = pin("out", 2, IPinType.Output);
		const caps = planPinLabelCaps(
			[input, action(input)],
			[action(output), output],
		);
		expect(Object.keys(caps).toSorted()).toEqual(["in", "out"]);
	});

	test("slots re-pair rows on collapsed nodes", () => {
		const input = pin("Mode", 3, IPinType.Input, { depends_on: ["src"] });
		const output = pin(LONG, 1, IPinType.Output);
		const byIndex = planPinLabelCaps([input], [output]);
		expect(byIndex[input.id]).toBe(PIN_LABEL_CAP_SOLO);
		const bySlot = planPinLabelCaps([input], [output], { [input.id]: 1 });
		expect(bySlot[input.id]).toBe(pinLabelNeed(input));
		expect(bySlot[output.id]).toBe(PIN_ROW_LABEL_BUDGET - pinLabelNeed(input));
	});

	test("caps never exceed the row budget", () => {
		const names = [
			"a",
			"Mode",
			"Archive After Processing Completes",
			LONG,
			"Send",
			"mmmmmmmmmmmmmmmmmmmm",
			"iiii",
			"Retry Count",
		];
		const inputs: IPin[] = [];
		const outputs: IPin[] = [];
		for (let i = 0; i < 24; i++) {
			const name = names[(i * 7) % names.length];
			const kindOverrides: Partial<IPin> =
				i % 4 === 0
					? { data_type: IVariableType.Boolean }
					: i % 4 === 1
						? { options: { valid_values: [name] } }
						: i % 4 === 2
							? { dynamic: true }
							: {};
			inputs.push(
				pin(`in_${i}`, (i % 6) + 1, IPinType.Input, {
					friendly_name: name,
					...kindOverrides,
				}),
			);
			outputs.push(
				pin(`out_${i}`, ((i * 5) % 6) + 1, IPinType.Output, {
					friendly_name: names[(i * 3) % names.length],
				}),
			);
		}
		const caps = planPinLabelCaps(inputs, outputs);
		const byRow = new Map<number, { capIn?: number; capOut?: number }>();
		for (const row of inputs) {
			byRow.set(row.index, { ...byRow.get(row.index), capIn: caps[row.id] });
		}
		for (const row of outputs) {
			byRow.set(row.index, { ...byRow.get(row.index), capOut: caps[row.id] });
		}
		for (const entry of byRow.values()) {
			expect(entry.capIn ?? 0).toBeLessThanOrEqual(PIN_LABEL_CAP_SOLO);
			expect(entry.capOut ?? 0).toBeLessThanOrEqual(PIN_LABEL_CAP_SOLO);
			if (entry.capIn !== undefined && entry.capOut !== undefined) {
				expect(entry.capIn + entry.capOut).toBeLessThanOrEqual(
					PIN_ROW_LABEL_BUDGET,
				);
			}
		}
	});
});
