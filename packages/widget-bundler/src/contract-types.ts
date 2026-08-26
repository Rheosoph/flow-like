// Producer-side (strict, canonical) mirror of packages/wasm/schema/src/widget.rs;
// @flow-like/widget-sdk ships the consumer-side (loose) mirror. The
// assignability assertion at the bottom keeps the two from drifting.

import type { WidgetContract as SdkWidgetContract } from "@flow-like/widget-sdk";

export type JsonValue =
	| string
	| number
	| boolean
	| null
	| JsonValue[]
	| { [key: string]: JsonValue };

export type JsonObject = { [key: string]: JsonValue };

export const CONTRACT_VERSION = 1;
export const WIDGET_PROTOCOL = "flw/1";

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
	default?: JsonValue;
	choices?: string[];
	min?: number;
	max?: number;
	schema?: JsonObject;
	optional?: boolean;
}

export interface ContractEvent {
	payloadSchema: JsonObject | null;
	description?: string;
}

export interface ContractQuery {
	argsSchema: JsonObject | null;
	resultSchema: JsonObject | null;
	description?: string;
}

export interface WidgetSizing {
	defaultHeight: number;
	resizable: boolean;
	maxHeight?: number;
}

export interface WidgetContract {
	contractVersion: number;
	id: string;
	inputs: Record<string, ContractInput>;
	events: Record<string, ContractEvent>;
	queries: Record<string, ContractQuery>;
	sizing: WidgetSizing;
}

export function isValidWidgetId(id: string): boolean {
	return (
		id.length > 0 &&
		!id.startsWith("-") &&
		!id.endsWith("-") &&
		/^[a-z0-9-]+$/.test(id)
	);
}

export function isValidMemberKey(key: string): boolean {
	return /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key);
}

function defaultMatchesType(value: JsonValue, input: ContractInput): boolean {
	switch (input.type) {
		case "string":
			return typeof value === "string";
		case "number":
			return typeof value === "number";
		case "integer":
			return typeof value === "number" && Number.isInteger(value);
		case "boolean":
			return typeof value === "boolean";
		case "enum":
			return typeof value === "string" && (input.choices ?? []).includes(value);
		case "json":
			return true;
	}
}

/** Mirrors `WidgetContract::validate` in packages/wasm/schema/src/widget.rs */
export function validateContract(contract: WidgetContract): string[] {
	const errors: string[] = [];

	if (
		contract.contractVersion === 0 ||
		contract.contractVersion > CONTRACT_VERSION
	) {
		errors.push(
			`Unsupported contract version ${contract.contractVersion} for widget '${contract.id}' (supported: 1..=${CONTRACT_VERSION})`,
		);
	}

	if (!isValidWidgetId(contract.id)) {
		errors.push(
			`Invalid widget id '${contract.id}': must be non-empty lowercase kebab-case ([a-z0-9-])`,
		);
	}

	for (const [key, input] of Object.entries(contract.inputs)) {
		if (!isValidMemberKey(key)) {
			errors.push(
				`Invalid input key '${key}' in widget '${contract.id}': must match [a-zA-Z_][a-zA-Z0-9_]*`,
			);
		}
		if (input.type === "enum" && (input.choices?.length ?? 0) === 0) {
			errors.push(
				`Enum input '${key}' in widget '${contract.id}' must declare non-empty choices`,
			);
		}
		if (
			input.min !== undefined &&
			input.max !== undefined &&
			input.min > input.max
		) {
			errors.push(
				`Input '${key}' in widget '${contract.id}' has min ${input.min} > max ${input.max}`,
			);
		}
		if (
			input.default !== undefined &&
			!defaultMatchesType(input.default, input)
		) {
			errors.push(
				`Default value for input '${key}' in widget '${contract.id}' does not match its declared type`,
			);
		}
	}

	for (const key of Object.keys(contract.events)) {
		if (!isValidMemberKey(key)) {
			errors.push(
				`Invalid event key '${key}' in widget '${contract.id}': must match [a-zA-Z_][a-zA-Z0-9_]*`,
			);
		}
	}

	for (const key of Object.keys(contract.queries)) {
		if (!isValidMemberKey(key)) {
			errors.push(
				`Invalid query key '${key}' in widget '${contract.id}': must match [a-zA-Z_][a-zA-Z0-9_]*`,
			);
		}
	}

	return errors;
}

function sortedEntries<T>(map: Record<string, T>): [string, T][] {
	return Object.entries(map).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

/**
 * Rebuild the contract with serde field order, BTreeMap-sorted keys, and the
 * same skip-serializing semantics as the Rust types, so the emitted JSON
 * matches what `serde_json` would produce.
 */
export function canonicalizeContract(contract: WidgetContract): WidgetContract {
	const inputs: Record<string, ContractInput> = {};
	for (const [key, input] of sortedEntries(contract.inputs)) {
		inputs[key] = {
			type: input.type,
			...(input.description !== undefined && {
				description: input.description,
			}),
			...(input.default !== undefined && { default: input.default }),
			...(input.choices !== undefined && { choices: input.choices }),
			...(input.min !== undefined && { min: input.min }),
			...(input.max !== undefined && { max: input.max }),
			...(input.schema !== undefined && { schema: input.schema }),
			...(input.optional === true && { optional: true }),
		};
	}

	const events: Record<string, ContractEvent> = {};
	for (const [key, event] of sortedEntries(contract.events)) {
		events[key] = {
			payloadSchema: event.payloadSchema ?? null,
			...(event.description !== undefined && {
				description: event.description,
			}),
		};
	}

	const queries: Record<string, ContractQuery> = {};
	for (const [key, query] of sortedEntries(contract.queries)) {
		queries[key] = {
			argsSchema: query.argsSchema ?? null,
			resultSchema: query.resultSchema ?? null,
			...(query.description !== undefined && {
				description: query.description,
			}),
		};
	}

	return {
		contractVersion: contract.contractVersion,
		id: contract.id,
		inputs,
		events,
		queries,
		sizing: {
			defaultHeight: contract.sizing.defaultHeight,
			resizable: contract.sizing.resizable,
			...(contract.sizing.maxHeight !== undefined && {
				maxHeight: contract.sizing.maxHeight,
			}),
		},
	};
}

export function contractToJson(contract: WidgetContract): string {
	return JSON.stringify(canonicalizeContract(contract), null, 2);
}

type AssertAssignable<_T extends U, U> = never;
export type ContractShapeMatchesSdk = AssertAssignable<
	WidgetContract,
	SdkWidgetContract
>;
