import { describe, expect, test } from "bun:test";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import { isSystemOneModelPin, resolvePinEditorKind } from "./pin-editor-kind";

function pin(name: string, overrides: Partial<IPin> = {}): IPin {
	return {
		id: name,
		index: 1,
		name,
		friendly_name: name,
		description: "",
		pin_type: IPinType.Input,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		connected_to: [],
		depends_on: [],
		...overrides,
	};
}

describe("resolvePinEditorKind", () => {
	test("outputs and wired inputs are plain labels", () => {
		expect(resolvePinEditorKind(pin("x", { pin_type: IPinType.Output }))).toBe(
			"label",
		);
		expect(resolvePinEditorKind(pin("x", { depends_on: ["src"] }))).toBe(
			"label",
		);
	});

	test("string pins with valid values are enums", () => {
		expect(
			resolvePinEditorKind(
				pin("mode", { options: { valid_values: ["a", "b"] } }),
			),
		).toBe("enum");
	});

	test("bit_id prefix routes to the bit selector", () => {
		expect(resolvePinEditorKind(pin("bit_id_x"))).toBe("bit");
		expect(
			resolvePinEditorKind(pin("bit_id_x", { value_type: IValueType.Array })),
		).toBe("plain");
	});

	test("widget_selector depends on the node", () => {
		expect(
			resolvePinEditorKind(pin("widget_selector"), "a2ui_instantiate_widget"),
		).toBe("widget");
		expect(resolvePinEditorKind(pin("widget_selector"), "other_node")).toBe(
			"plain",
		);
	});

	test("the payment tax code gets the Stripe category picker", () => {
		expect(
			resolvePinEditorKind(pin("product_tax_code"), "request_payment"),
		).toBe("taxCode");
		expect(resolvePinEditorKind(pin("product_tax_code"), "other_node")).toBe(
			"plain",
		);
		expect(
			resolvePinEditorKind(
				pin("product_tax_code", { depends_on: ["src"] }),
				"request_payment",
			),
		).toBe("label");
	});

	test("booleans route to the checkbox", () => {
		expect(
			resolvePinEditorKind(pin("flag", { data_type: IVariableType.Boolean })),
		).toBe("boolean");
	});
});

test.each([
	"ai_systemone_invoke",
	"ai_systemone_noul",
	"ai_systemone_choice",
	"ai_systemone_score",
])("%s model references use the SystemOne Bit selector", (nodeName) => {
	const model = pin("model", { data_type: IVariableType.String });
	expect(resolvePinEditorKind(model, nodeName)).toBe("bit");
	expect(isSystemOneModelPin(model, nodeName)).toBe(true);
	expect(isSystemOneModelPin(pin("instructions"), nodeName)).toBe(false);
	expect(resolvePinEditorKind(model, "another_node")).toBe("plain");
	expect(isSystemOneModelPin(model, "another_node")).toBe(false);
	expect(
		resolvePinEditorKind({ ...model, depends_on: ["source"] }, nodeName),
	).toBe("label");
});
