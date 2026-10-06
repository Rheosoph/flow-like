import { describe, expect, test } from "bun:test";
import {
	FIELD_KEY_SEPARATOR,
	type FieldKind,
	type ShortcutGroup,
	type WorkbenchField,
} from "../contracts";
import { dateLocaleOf } from "./dates";
import { type ShortcutOptions, shortcutGroups } from "./shortcuts";

function field(
	name: string,
	kind: FieldKind,
	extra: Partial<WorkbenchField> = {},
) {
	const made: WorkbenchField = {
		key: name,
		name,
		label: name,
		help: null,
		kind,
		dataType: "String",
		valueType: "Normal",
		required: false,
		sensitive: false,
		defaultOmitted: false,
		defaultValue: "",
		hasDefault: false,
		options: null,
		range: null,
		step: null,
		integer: false,
		fileMode: null,
		itemKind: null,
		dateFormat: null,
		props: [],
		short: false,
		index: 0,
		...extra,
	};
	return made;
}

/** Extract invoice (FLP_FORMS.medium): nine fields. */
const MEDIUM = [
	field("invoice_file", "file", { fileMode: "flowpath" }),
	field("supporting_documents", "files", { fileMode: "flowpath" }),
	field("vendor_name", "text"),
	field("invoice_date", "date"),
	field("expected_total", "number"),
	field("max_pages", "number", { integer: true }),
	field("run_ocr", "bool"),
	field("cost_centers", "chips", { itemKind: "text" }),
	field("payment_terms", "pairs"),
];

const GB = dateLocaleOf("en-GB");

const MAIN: ShortcutOptions = {
	anchorIso: "2026-09-22",
	todayIso: "2026-09-30",
	mac: true,
	built: { presets: true, numbers: true },
	dateLocale: GB,
};

const actions = (groups: readonly ShortcutGroup[]) =>
	groups.map((group) => [group.id, group.rows.map((row) => row.action)]);

describe("shortcutGroups", () => {
	test("the medium form with every should built (spec §8 shortcuts)", () => {
		expect(actions(shortcutGroups(MEDIUM, MAIN))).toEqual([
			["run", ["run", "runLeave", "stop"]],
			["move", ["enterNext", "tabNext", "runTabs"]],
			[
				"faster",
				[
					"recent",
					"acceptSuggestion",
					"resetField",
					"chooseFiles",
					"replaceFile",
					"removeFile",
					"undo",
					"calendar",
					"nudge",
					"presets",
					"savePreset",
				],
			],
			["dates", ["dateDay", "dateDayMonth", "dateFull", "dateWords"]],
		]);
	});

	test("⌘ chips on a Mac", () => {
		const [run, move, faster] = shortcutGroups(MEDIUM, MAIN);
		expect(run.rows.map((row) => row.keys)).toEqual([["⌘↵"], ["⇧⌘↵"], ["⌘."]]);
		expect(move.rows[1].keys).toEqual(["Tab", "⇧Tab"]);
		expect(faster.rows.find((row) => row.action === "nudge")?.keys).toEqual([
			"↑",
			"↓",
			"⇧↑",
			"⇧↓",
		]);
	});

	test("Ctrl words elsewhere", () => {
		const [run, , faster] = shortcutGroups(MEDIUM, { ...MAIN, mac: false });
		expect(run.rows.map((row) => row.keys)).toEqual([
			["Ctrl+Enter"],
			["Ctrl+Shift+Enter"],
			["Ctrl+."],
		]);
		const keysOf = (action: string) =>
			faster.rows.find((row) => row.action === action)?.keys;
		expect(keysOf("resetField")).toEqual(["Ctrl+Shift+Backspace"]);
		expect(keysOf("calendar")).toEqual(["Alt+↓"]);
		expect(keysOf("removeFile")).toEqual(["Backspace"]);
	});

	test("typing dates reads the examples against the anchor in the viewer's order", () => {
		const dates = shortcutGroups(MEDIUM, MAIN).at(-1);
		expect(dates?.rows).toEqual([
			{ action: "dateDay", keys: ["18"], example: "2026-09-18" },
			{ action: "dateDayMonth", keys: ["18/9"], example: "2026-09-18" },
			{ action: "dateFull", keys: ["18/9/26"], example: "2026-09-18" },
			{ action: "dateWords", keys: ["today", "yesterday", "tomorrow"] },
		]);
	});

	test("month first, year first and German date rows", () => {
		const rowsIn = (lang: string) =>
			shortcutGroups(MEDIUM, { ...MAIN, dateLocale: dateLocaleOf(lang) })
				.at(-1)
				?.rows.map((row) => row.keys.join(" "));
		expect(rowsIn("en-US")).toEqual([
			"18",
			"9/18",
			"9/18/26",
			"today yesterday tomorrow",
		]);
		expect(rowsIn("ja")).toEqual(["18", "9/18", "2026/9/18", "今日 昨日 明日"]);
		expect(rowsIn("de")).toEqual([
			"18",
			"18.9",
			"18.9.26",
			"heute gestern morgen",
		]);
	});

	test("a form without fields gets Run and Move around only", () => {
		const groups = shortcutGroups([], { ...MAIN, mac: false });
		expect(actions(groups)).toEqual([
			["run", ["run", "stop"]],
			["move", ["runTabs"]],
		]);
		expect(groups[0].rows.map((row) => row.keys)).toEqual([
			["Ctrl+Enter"],
			["Ctrl+."],
		]);
	});

	test("the S1 and S3 rows wait until they are built", () => {
		const faster = shortcutGroups(MEDIUM, {
			...MAIN,
			built: { presets: false, numbers: false },
		}).find((group) => group.id === "faster");
		const rows = faster?.rows.map((row) => row.action) ?? [];
		expect(rows).not.toContain("nudge");
		expect(rows).not.toContain("presets");
		expect(rows).not.toContain("savePreset");
	});

	test('"/" needs 12 or more fields', () => {
		const twelve = Array.from({ length: 12 }, (_, index) =>
			field(`field_${index}`, "bool"),
		);
		const moveOf = (fields: readonly WorkbenchField[]) =>
			shortcutGroups(fields, MAIN)
				.find((group) => group.id === "move")
				?.rows.map((row) => row.action);
		expect(moveOf(twelve)).toContain("filter");
		expect(moveOf(twelve.slice(1))).not.toContain("filter");
	});

	test("rows follow the kinds in the form, object properties included", () => {
		const group = field("period", "group", {
			props: [
				field("from", "date", { key: `period${FIELD_KEY_SEPARATOR}from` }),
				field("limit", "number", { key: `period${FIELD_KEY_SEPARATOR}limit` }),
			],
		});
		const groups = shortcutGroups([group], MAIN);
		const faster = groups.find((candidate) => candidate.id === "faster");
		expect(faster?.rows.map((row) => row.action)).toEqual([
			"resetField",
			"undo",
			"calendar",
			"nudge",
			"presets",
			"savePreset",
		]);
		expect(groups.at(-1)?.id).toBe("dates");
	});

	test("lists of numbers get no recent-values rows; lists of texts do", () => {
		const rowsOf = (itemKind: WorkbenchField["itemKind"]) =>
			shortcutGroups([field("tags", "chips", { itemKind })], MAIN)
				.find((group) => group.id === "faster")
				?.rows.map((row) => row.action);
		expect(rowsOf("number")).not.toContain("recent");
		expect(rowsOf("text")).toContain("recent");
	});
});
