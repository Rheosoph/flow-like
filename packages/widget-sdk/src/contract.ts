export const CONTRACT_VERSION = 1;

export type JsonSchema = Record<string, unknown>;

export type ContractInputType =
	| "string"
	| "number"
	| "integer"
	| "boolean"
	| "enum"
	| "json";

export interface ContractInput {
	type: ContractInputType;
	description?: string;
	default?: unknown;
	choices?: string[];
	min?: number;
	max?: number;
	schema?: JsonSchema;
	optional?: boolean;
}

export interface ContractEvent {
	payloadSchema?: JsonSchema | null;
	description?: string;
}

export interface ContractQuery {
	argsSchema?: JsonSchema | null;
	resultSchema?: JsonSchema | null;
	description?: string;
}

export interface WidgetSizing {
	defaultHeight?: number;
	resizable?: boolean;
	maxHeight?: number;
}

export interface WidgetContract {
	contractVersion: number;
	id: string;
	inputs?: Record<string, ContractInput>;
	events?: Record<string, ContractEvent>;
	queries?: Record<string, ContractQuery>;
	sizing?: WidgetSizing;
}

export function contractDefaults(
	contract: WidgetContract | null | undefined,
): Record<string, unknown> {
	const defaults: Record<string, unknown> = {};
	if (!contract?.inputs) return defaults;
	for (const [key, input] of Object.entries(contract.inputs)) {
		if (input.default !== undefined) defaults[key] = input.default;
	}
	return defaults;
}
