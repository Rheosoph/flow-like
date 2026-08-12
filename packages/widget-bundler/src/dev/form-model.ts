import type {
	ContractInput,
	JsonObject,
	JsonValue,
	WidgetContract,
} from "../contract-types";

export type PropsFormControl =
	| { kind: "text" }
	| { kind: "number"; integer: boolean; min?: number; max?: number }
	| { kind: "checkbox" }
	| { kind: "select"; choices: string[] }
	| { kind: "json"; schema?: JsonObject };

export interface PropsFormField {
	key: string;
	label: string;
	description?: string;
	optional: boolean;
	default?: JsonValue;
	control: PropsFormControl;
}

function controlFor(input: ContractInput): PropsFormControl {
	switch (input.type) {
		case "string":
			return { kind: "text" };
		case "number":
		case "integer":
			return {
				kind: "number",
				integer: input.type === "integer",
				...(input.min !== undefined && { min: input.min }),
				...(input.max !== undefined && { max: input.max }),
			};
		case "boolean":
			return { kind: "checkbox" };
		case "enum":
			return { kind: "select", choices: input.choices ?? [] };
		case "json":
			return {
				kind: "json",
				...(input.schema !== undefined && { schema: input.schema }),
			};
	}
}

/**
 * Map a widget contract to the harness props panel's form field descriptors
 * (one control per contract input, in contract order).
 */
export function derivePropsFormModel(
	contract: WidgetContract,
): PropsFormField[] {
	return Object.entries(contract.inputs).map(([key, input]) => ({
		key,
		label: key,
		...(input.description !== undefined && { description: input.description }),
		optional: input.optional === true,
		...(input.default !== undefined && { default: input.default }),
		control: controlFor(input),
	}));
}
