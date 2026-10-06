import { describe, expect, test } from "bun:test";
import {
	FORM_LIMITS,
	type Preset,
	type PresetDraft,
	type SessionEffect,
} from "../contracts";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import { focusedKey, sessionDriver } from "./reduce-driver";

const from = (name: FixtureName) =>
	sessionDriver(fixture(name), fixtureClock(name));

const savedPresets = (effects: readonly SessionEffect[]) =>
	effects.flatMap((effect) =>
		effect.type === "persistPreset" ? [effect.preset] : [],
	);

const draft = (parts: Partial<PresetDraft>): PresetDraft => ({
	name: "Nordwind Logistik GmbH",
	openDefault: false,
	ticked: ["vendor_name"],
	fromRunId: null,
	replaceId: null,
	...parts,
});

const presetNamed = (presets: readonly Preset[], name: string) =>
	presets.find((preset) => preset.name === name);

describe("applying a preset (spec S1)", () => {
	test("fields at the old starting value move, '{name} applied.' with Undo; the menu closes", () => {
		const driver = from("presets");
		driver.command({ type: "applyPreset", presetId: "preset-alpenfracht" });
		const { rail, view, memory } = driver.state;
		expect(rail.values.vendor_name).toBe("Alpenfracht AG");
		expect(rail.values.max_pages).toBe("60");
		expect(rail.values.expected_total).toBe("11769.10");
		expect(rail.activePresetId).toBe("preset-alpenfracht");
		expect(view.overlay).toBeNull();
		expect(view.message?.message).toEqual({
			kind: "presetApplied",
			name: "Alpenfracht AG",
			misfit: 0,
		});
		expect(view.message?.undo).toBe(true);
		expect(presetNamed(memory.presets, "Alpenfracht AG")?.lastUsedAt).toBe(
			driver.now,
		);
		expect(savedPresets(driver.last)).toHaveLength(1);
		expect(driver.state.view.focus?.target).toEqual({ kind: "presetButton" });
		driver.command({ type: "undo" });
		expect(driver.state.rail.values.vendor_name).toBe("Nordwind Logistik GmbH");
		expect(driver.state.rail.values.max_pages).toBe("20");
		expect(driver.state.rail.activePresetId).toBe("preset-nordwind");
	});

	test("an edited field keeps its value unless the preset sets it; files never change", () => {
		const driver = from("presets");
		driver.command({ type: "setValue", key: "max_pages", value: "33" });
		driver.command({ type: "setValue", key: "expected_total", value: "5" });
		const invoice = driver.state.rail.values.invoice_file;
		driver.command({ type: "applyPreset", presetId: "preset-alpenfracht" });
		expect(driver.state.rail.values.max_pages).toBe("60");
		expect(driver.state.rail.values.expected_total).toBe("5");
		expect(driver.state.rail.values.invoice_file).toBe(invoice);
	});

	test("a saved input that no longer fits is counted; the cursor goes to the first empty required field", () => {
		const driver = from("idle");
		const state = driver.state;
		const odd: Preset = {
			id: "preset-odd",
			name: "Odd",
			digit: 1,
			sets: { vendor_name: "X", removed_field: "y" },
			kinds: { vendor_name: "text", removed_field: "text" },
			openDefault: false,
			createdAt: 1,
			updatedAt: 1,
			lastUsedAt: null,
		};
		const withPreset = sessionDriver(
			{ ...state, memory: { ...state.memory, presets: [odd] } },
			fixtureClock("idle"),
		);
		withPreset.command({ type: "applyPreset", presetId: "preset-odd" });
		const message = withPreset.state.view.message?.message;
		expect(message?.kind === "presetApplied" && message.misfit).toBe(1);
		expect(focusedKey(withPreset.state)).toBe("invoice_file");
	});
});

describe("saving, updating, deleting (spec S1)", () => {
	test("save from the rail: the lowest free digit, 'Saved as {name}.', the preset is active", () => {
		const driver = from("preset-save");
		driver.command({ type: "savePreset", draft: draft({ openDefault: true }) });
		const [preset] = driver.state.memory.presets;
		expect(preset.name).toBe("Nordwind Logistik GmbH");
		expect(preset.digit).toBe(1);
		expect(preset.sets).toEqual({ vendor_name: "Nordwind Logistik GmbH" });
		expect(preset.openDefault).toBe(true);
		expect(driver.state.rail.activePresetId).toBe(preset.id);
		expect(driver.state.view.overlay).toBeNull();
		expect(driver.state.view.message?.message).toEqual({
			kind: "presetSaved",
			name: "Nordwind Logistik GmbH",
		});
		expect(savedPresets(driver.last)).toEqual([preset]);
	});

	test("the same name in another case replaces it and keeps its digit; On open stays unique", () => {
		const driver = from("presets");
		driver.command({
			type: "savePreset",
			draft: draft({
				name: "alpenfracht ag",
				openDefault: true,
				ticked: ["max_pages"],
			}),
		});
		const { presets } = driver.state.memory;
		expect(presets).toHaveLength(2);
		const replaced = presets.find(
			(preset) => preset.id === "preset-alpenfracht",
		);
		expect(replaced?.name).toBe("alpenfracht ag");
		expect(replaced?.digit).toBe(1);
		expect(replaced?.sets).toEqual({ max_pages: "20" });
		expect(replaced?.openDefault).toBe(true);
		expect(
			presets.find((preset) => preset.id === "preset-nordwind")?.openDefault,
		).toBe(false);
		expect(
			savedPresets(driver.last)
				.map((preset) => preset.id)
				.sort(),
		).toEqual(["preset-alpenfracht", "preset-nordwind"]);
	});

	test("save a run's inputs from its menu: its copy, the rail's preset unchanged", () => {
		const driver = from("done");
		driver.command({ type: "setValue", key: "vendor_name", value: "Other" });
		driver.command({
			type: "savePreset",
			draft: draft({ fromRunId: "run-14", name: "Run 14" }),
		});
		const preset = presetNamed(driver.state.memory.presets, "Run 14");
		expect(preset?.sets).toEqual({ vendor_name: "Nordwind Logistik GmbH" });
		expect(driver.state.rail.activePresetId).toBeNull();
	});

	test("no more than 30 presets per form; an empty name saves nothing", () => {
		const base = fixture("idle");
		const presets = Array.from(
			{ length: FORM_LIMITS.presetsPerForm },
			(_, index) => ({
				id: `p${index}`,
				name: `Preset ${index}`,
				digit: 0,
				sets: {},
				kinds: {},
				openDefault: false,
				createdAt: index,
				updatedAt: index,
				lastUsedAt: null,
			}),
		);
		const driver = sessionDriver(
			{ ...base, memory: { ...base.memory, presets } },
			fixtureClock("idle"),
		);
		driver.command({ type: "savePreset", draft: draft({ name: "One more" }) });
		expect(driver.state.memory.presets).toHaveLength(
			FORM_LIMITS.presetsPerForm,
		);
		driver.command({ type: "savePreset", draft: draft({ name: "   " }) });
		expect(driver.last).toEqual([]);
	});

	test("Update writes the current values of the fields it sets, with Undo", () => {
		const driver = from("presets");
		driver.command({ type: "applyPreset", presetId: "preset-alpenfracht" });
		driver.command({ type: "setValue", key: "max_pages", value: "70" });
		driver.command({ type: "setValue", key: "expected_total", value: "9" });
		driver.command({ type: "updatePreset", presetId: "preset-alpenfracht" });
		const updated = presetNamed(driver.state.memory.presets, "Alpenfracht AG");
		expect(updated?.sets).toEqual({
			vendor_name: "Alpenfracht AG",
			max_pages: "70",
		});
		expect(driver.state.view.message?.message).toEqual({
			kind: "presetUpdated",
			name: "Alpenfracht AG",
		});
		driver.command({ type: "undo" });
		expect(
			presetNamed(driver.state.memory.presets, "Alpenfracht AG")?.sets
				.max_pages,
		).toBe("60");
		expect(savedPresets(driver.last)[0].sets.max_pages).toBe("60");
	});

	test("Delete with Undo brings it back with its digit and as the active preset", () => {
		const driver = from("presets");
		driver.command({ type: "deletePreset", presetId: "preset-nordwind" });
		expect(driver.state.memory.presets.map((preset) => preset.id)).toEqual([
			"preset-alpenfracht",
		]);
		expect(driver.state.rail.activePresetId).toBeNull();
		expect(driver.last).toContainEqual({
			type: "deletePreset",
			presetId: "preset-nordwind",
		});
		driver.command({ type: "undo" });
		const back = driver.state.memory.presets.find(
			(preset) => preset.id === "preset-nordwind",
		);
		expect(back?.digit).toBe(2);
		expect(driver.state.rail.activePresetId).toBe("preset-nordwind");
	});

	test("Reset to {name}: the fields it sets go back to its values, with Undo", () => {
		const driver = from("presets");
		driver.command({ type: "setValue", key: "vendor_name", value: "Someone" });
		driver.command({ type: "setValue", key: "max_pages", value: "99" });
		driver.command({ type: "resetToPreset" });
		expect(driver.state.rail.values.vendor_name).toBe("Nordwind Logistik GmbH");
		expect(driver.state.rail.values.max_pages).toBe("99");
		expect(driver.state.view.message?.message).toEqual({
			kind: "resetTo",
			presetName: "Nordwind Logistik GmbH",
		});
		driver.command({ type: "undo" });
		expect(driver.state.rail.values.vendor_name).toBe("Someone");
	});
});
