/**
 * Drift-prevention tests: the validator's known-prop tables must stay in sync
 * with the A2UIComponent union in components/a2ui/types.ts (mirrored by the
 * compile-time-checked manifest in components/a2ui/component-prop-manifest.ts).
 */
import { describe, expect, test } from "bun:test";
import {
	COMPONENT_BASE_PROPS,
	COMPONENT_PROPS,
} from "../a2ui/component-prop-manifest";
import { BASE_PROPS, KNOWN_PROPS } from "./validateComponents";

/** The only props the validator may accept beyond the types.ts interfaces. */
const ALLOWED_RUNTIME_ONLY_PROPS: Record<string, readonly string[]> = {
	widgetInstance: ["inlineWidgetDef"],
};

describe("KNOWN_PROPS drift protection", () => {
	test("has an entry for every component type in the A2UIComponent union", () => {
		const manifestTypes = Object.keys(COMPONENT_PROPS).sort();
		const validatorTypes = Object.keys(KNOWN_PROPS).sort();
		expect(validatorTypes).toEqual(manifestTypes);
	});

	test("accepts every prop declared on each component interface", () => {
		for (const [type, props] of Object.entries(COMPONENT_PROPS)) {
			const known = KNOWN_PROPS[type];
			expect(known).toBeDefined();
			const missing = props.filter((prop) => !known?.has(prop));
			expect(`${type}: ${missing.join(",")}`).toBe(`${type}: `);
		}
	});

	test("accepts no props beyond the interface plus declared runtime-only extras", () => {
		for (const [type, known] of Object.entries(KNOWN_PROPS)) {
			const allowed = new Set<string>([
				...(COMPONENT_PROPS[type as keyof typeof COMPONENT_PROPS] ?? []),
				...(ALLOWED_RUNTIME_ONLY_PROPS[type] ?? []),
			]);
			const extras = [...known].filter((prop) => !allowed.has(prop));
			expect(`${type}: ${extras.join(",")}`).toBe(`${type}: `);
		}
	});

	test("documented interactive/planning components are not rejected", () => {
		for (const type of [
			"feedback",
			"appLink",
			"calendar",
			"gantt",
			"userProfile",
			"voiceInput",
			"geoMap",
			"tableRow",
			"tableCell",
		]) {
			expect(KNOWN_PROPS[type]).toBeDefined();
			expect(KNOWN_PROPS[type].size).toBeGreaterThan(0);
		}
	});

	test("nivoChart accepts its current chart props", () => {
		for (const prop of ["title", "indexBy", "keys", "axisTop", "axisRight"]) {
			expect(KNOWN_PROPS.nivoChart.has(prop)).toBe(true);
		}
	});

	test("dialogue uses the current prop names, not the stale ones", () => {
		expect(KNOWN_PROPS.dialogue.has("typewriterSpeed")).toBe(true);
		expect(KNOWN_PROPS.dialogue.has("speakerPortraitId")).toBe(true);
		expect(KNOWN_PROPS.dialogue.has("speed")).toBe(false);
		expect(KNOWN_PROPS.dialogue.has("portrait")).toBe(false);
	});
});

describe("BASE_PROPS drift protection", () => {
	test("matches ComponentBase (including hidden) plus the type discriminant", () => {
		expect([...BASE_PROPS].sort()).toEqual(
			["type", ...COMPONENT_BASE_PROPS].sort(),
		);
		expect(BASE_PROPS.has("hidden")).toBe(true);
	});
});
