import { describe, expect, test } from "bun:test";
import {
	compareModelCapability,
	getModelCapability,
	meetsModelCapabilityMinimum,
} from "./model-capabilities";

describe("model capabilities", () => {
	test("distinguishes missing measurements from measured zero", () => {
		const unavailable = {
			model_classification: { reasoning: 0, coding: 0, cost: 0.7, speed: 0.9 },
			classification_unavailable_fields: ["reasoning"],
		};
		expect(getModelCapability(unavailable, "reasoning")).toBeNull();
		expect(getModelCapability(unavailable, "coding")).toBe(0);
		expect(getModelCapability(unavailable, "cost")).toBe(0.7);
		expect(getModelCapability(unavailable, "speed")).toBe(0.9);
		expect(getModelCapability(unavailable, "creativity")).toBeNull();
	});

	test("sorts high scores first and unknown scores after measured zero", () => {
		const models = [
			{
				id: "unknown",
				parameters: {
					model_classification: { coding: 0 },
					classification_unavailable_fields: ["coding"],
				},
			},
			{ id: "zero", parameters: { model_classification: { coding: 0 } } },
			{ id: "high", parameters: { model_classification: { coding: 0.9 } } },
			{ id: "medium", parameters: { model_classification: { coding: 0.5 } } },
			{ id: "missing", parameters: undefined },
		];
		expect(
			models
				.sort((a, b) =>
					compareModelCapability(a.parameters, b.parameters, "coding"),
				)
				.map((model) => model.id),
		).toEqual(["high", "medium", "zero", "unknown", "missing"]);
	});

	test("positive filters exclude unavailable and absent measurements", () => {
		const unavailable = {
			model_classification: { reasoning: 0.9 },
			classification_unavailable_fields: ["reasoning"],
		};
		for (const parameters of [
			unavailable,
			undefined,
			{ model_classification: {} },
		]) {
			expect(meetsModelCapabilityMinimum(parameters, "reasoning", 0.1)).toBe(
				false,
			);
			expect(meetsModelCapabilityMinimum(parameters, "reasoning", 0)).toBe(
				true,
			);
		}
		const measured = { model_classification: { reasoning: 0.5 } };
		expect(meetsModelCapabilityMinimum(measured, "reasoning", 0.5)).toBe(true);
		expect(meetsModelCapabilityMinimum(measured, "reasoning", 0.6)).toBe(false);
	});

	test("honors explicit cost and speed unavailability without changing other values", () => {
		const params = {
			model_classification: { cost: 0, speed: 0.8, coding: 0.4 },
			classification_unavailable_fields: ["cost", "speed"],
		};
		for (const field of ["cost", "speed"]) {
			expect(getModelCapability(params, field)).toBeNull();
			expect(
				compareModelCapability(
					params,
					{ model_classification: { [field]: 0 } },
					field,
				),
			).toBeGreaterThan(0);
			expect(meetsModelCapabilityMinimum(params, field, 0.1)).toBe(false);
		}
		expect(getModelCapability(params, "coding")).toBe(0.4);
	});

	test("does not sort malformed measurements as real scores", () => {
		for (const value of [
			undefined,
			null,
			"0.5",
			Number.NaN,
			Number.POSITIVE_INFINITY,
			-0.1,
			1.1,
		]) {
			expect(
				getModelCapability(
					{ model_classification: { coding: value } },
					"coding",
				),
			).toBeNull();
		}
		for (const parameters of [
			null,
			[],
			"invalid",
			{ model_classification: [] },
		]) {
			expect(getModelCapability(parameters, "coding")).toBeNull();
		}
	});
});
