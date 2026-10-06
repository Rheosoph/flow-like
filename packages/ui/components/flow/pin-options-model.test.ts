import { describe, expect, test } from "bun:test";
import type { IPinOptions } from "../../lib/schema/flow/pin";
import { IVariableType } from "../../lib/schema/flow/variable";
import {
	type PinOptionsTarget,
	appendValues,
	buildPinOptionsResult,
	draftFromPin,
	hasPinOptionSections,
	pinOptionSections,
	schemaStatus,
	splitValueDraft,
	validatePinOptionsDraft,
} from "./pin-options-model";

const pin = (
	data_type: IVariableType,
	options: IPinOptions | null = null,
	schema: string | null = null,
): PinOptionsTarget => ({ data_type, options, schema });

describe("pinOptionSections", () => {
	test("offers allowed values only for string pins", () => {
		expect(pinOptionSections(pin(IVariableType.String)).validValues).toBe(true);
		expect(pinOptionSections(pin(IVariableType.Struct)).validValues).toBe(
			false,
		);
	});

	test("offers a range for numbers and a step only for floats", () => {
		const integer = pinOptionSections(pin(IVariableType.Integer));
		const float = pinOptionSections(pin(IVariableType.Float));
		expect([integer.range, integer.step]).toEqual([true, false]);
		expect([float.range, float.step]).toEqual([true, true]);
		expect(pinOptionSections(pin(IVariableType.String)).range).toBe(false);
	});

	test("keeps a section that already holds a value so it can be cleared", () => {
		const sections = pinOptionSections(
			pin(IVariableType.Boolean, { valid_values: ["a"], range: [0, 1] }),
		);
		expect(sections.validValues).toBe(true);
		expect(sections.range).toBe(true);
	});

	test("offers value shape only for generic pins", () => {
		expect(pinOptionSections(pin(IVariableType.Generic)).valueShape).toBe(true);
		expect(pinOptionSections(pin(IVariableType.String)).valueShape).toBe(false);
	});

	test("execution pins have nothing to configure", () => {
		expect(hasPinOptionSections(pin(IVariableType.Execution))).toBe(false);
	});
});

describe("value chips", () => {
	test("a trailing comma completes the value instead of vanishing", () => {
		expect(splitValueDraft("low,")).toEqual({ complete: ["low"], rest: "" });
	});

	test("pasted lists split on commas and newlines", () => {
		expect(splitValueDraft("a, b\nc, d")).toEqual({
			complete: ["a", " b", "c"],
			rest: "d",
		});
	});

	test("text without a separator stays in the input without leading spaces", () => {
		expect(splitValueDraft(" med ")).toEqual({ complete: [], rest: "med " });
	});

	test("appending trims, drops empties and skips duplicates", () => {
		expect(appendValues(["low"], [" low ", "", " high", "high"])).toEqual([
			"low",
			"high",
		]);
	});
});

describe("validation", () => {
	const numberPin = pin(IVariableType.Float);
	const sections = pinOptionSections(numberPin);
	const draft = draftFromPin(numberPin);

	test("a range needs both bounds", () => {
		expect(validatePinOptionsDraft({ ...draft, min: "1" }, sections)).toContain(
			"rangeIncomplete",
		);
	});

	test("a range must not be inverted", () => {
		expect(
			validatePinOptionsDraft({ ...draft, min: "5", max: "1" }, sections),
		).toContain("rangeInverted");
	});

	test("a step must be positive", () => {
		expect(
			validatePinOptionsDraft({ ...draft, step: "0" }, sections),
		).toContain("stepNotPositive");
	});

	test("JSON-looking schema text must parse", () => {
		const structPin = pin(IVariableType.Struct);
		expect(
			validatePinOptionsDraft(
				{ ...draftFromPin(structPin), schema: '{"type":' },
				pinOptionSections(structPin),
			),
		).toEqual(["schemaInvalidJson"]);
	});
});

describe("schemaStatus", () => {
	test("reads the schema title", () => {
		expect(schemaStatus('{"title":"FlowPath","type":"object"}')).toEqual({
			kind: "json",
			title: "FlowPath",
		});
	});

	test("treats non-JSON text as an identifier", () => {
		expect(schemaStatus("my.schema.Identifier")).toEqual({
			kind: "identifier",
		});
	});
});

describe("buildPinOptionsResult", () => {
	test("an untouched pin keeps its options and schema", () => {
		const target = pin(
			IVariableType.Struct,
			{ optional: true },
			'{"title":"FlowPath"}',
		);
		expect(buildPinOptionsResult(target, draftFromPin(target))).toEqual({
			options: { optional: true },
		});
	});

	test("a pin without options stays without options", () => {
		const target = pin(IVariableType.String);
		expect(buildPinOptionsResult(target, draftFromPin(target))).toEqual({
			options: null,
		});
	});

	test("a pending value is saved with the chips", () => {
		const target = pin(IVariableType.String);
		const draft = {
			...draftFromPin(target),
			validValues: ["low"],
			pendingValue: "high ",
		};
		expect(buildPinOptionsResult(target, draft).options?.valid_values).toEqual([
			"low",
			"high",
		]);
	});

	test("removing every value clears the list", () => {
		const target = pin(IVariableType.String, { valid_values: ["a"] });
		const draft = { ...draftFromPin(target), validValues: [] };
		expect(buildPinOptionsResult(target, draft).options?.valid_values).toBe(
			null,
		);
	});

	test("the range is saved as a pair", () => {
		const target = pin(IVariableType.Integer);
		const draft = { ...draftFromPin(target), min: "-2", max: "10" };
		expect(buildPinOptionsResult(target, draft).options?.range).toEqual([
			-2, 10,
		]);
	});

	test("an untouched flag keeps an explicit false", () => {
		const target = pin(IVariableType.Struct, { enforce_schema: false });
		expect(buildPinOptionsResult(target, draftFromPin(target)).options).toEqual(
			{ enforce_schema: false },
		);
	});

	test("a schema ref stays a ref unless the schema text changes", () => {
		const schema = '{"title":"FlowPath","type":"object"}';
		const target = pin(IVariableType.Struct, null, "ref-1");
		const refs = { "ref-1": schema };
		const draft = draftFromPin(target, refs);
		expect(draft.schema).toContain('\n  "title": "FlowPath"');
		expect(buildPinOptionsResult(target, draft, refs).schema).toBeUndefined();
		expect(
			buildPinOptionsResult(
				target,
				{ ...draft, schema: '{ "title": "Other" }' },
				refs,
			).schema,
		).toBe('{"title":"Other"}');
	});

	test("clearing the schema text clears the schema", () => {
		const target = pin(IVariableType.Struct, null, "my.schema.Identifier");
		const draft = { ...draftFromPin(target), schema: "  " };
		expect(buildPinOptionsResult(target, draft).schema).toBeNull();
	});
});
