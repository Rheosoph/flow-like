import { describe, expect, test } from "bun:test";
import {
	NO_RESTRICTIONS,
	RESTRICTION_PRESETS,
	detectPreset,
	holds,
	normalizeFolder,
	normalizeRestrictions,
	presetRestrictions,
	setFolders,
	setScopeList,
	togglePermission,
} from "./bot-restrictions-model";

describe("bot restriction presets", () => {
	test("each preset is detected as itself and a tweak is custom", () => {
		for (const preset of RESTRICTION_PRESETS) {
			expect(detectPreset(presetRestrictions(preset))).toBe(preset);
		}
		const tweaked = togglePermission(
			presetRestrictions("read_only"),
			"write_files",
			true,
		);
		expect(detectPreset(tweaked)).toBeNull();
		expect(detectPreset(setFolders(NO_RESTRICTIONS, ["reports"]))).toBeNull();
	});

	test("read only does not call or change anything", () => {
		const readOnly = presetRestrictions("read_only");
		expect(holds(readOnly, "read_files")).toBe(true);
		expect(holds(readOnly, "execute_events")).toBe(false);
		expect(
			holds(presetRestrictions("read_and_call_events"), "execute_events"),
		).toBe(true);
		expect(holds(NO_RESTRICTIONS, "write_files")).toBe(true);
	});

	test("one switch narrows the owner's whole role", () => {
		const narrowed = togglePermission(NO_RESTRICTIONS, "write_files", false);
		expect(narrowed.permissions).not.toBeNull();
		expect(holds(narrowed, "write_files")).toBe(false);
		expect(holds(narrowed, "read_files")).toBe(true);
	});

	test("allow-lists and folders", () => {
		const limited = setScopeList(NO_RESTRICTIONS, "tables", ["sales"]);
		expect(limited.scopes.tables).toEqual(["sales"]);
		expect(setScopeList(limited, "tables", null).scopes.tables).toBeNull();
		expect(normalizeFolder(" /reports/2026/ ")).toBe("reports/2026");
		expect(normalizeRestrictions(undefined)).toEqual(NO_RESTRICTIONS);
	});
});
