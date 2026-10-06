import {
	type DateLocale,
	FORM_LIMITS,
	type FieldKind,
	type ShortcutActionId,
	type ShortcutGroupId,
	type ShortcutRow,
	type WorkbenchField,
} from "../contracts";
import { readDate } from "./dates";

/** What a row needs from the form before the sheet lists it. */
type ShortcutNeed =
	| "always"
	| "fields"
	| "many"
	| "text"
	| "file"
	| "date"
	| "numbers"
	| "presets";

interface ShortcutSpec {
	readonly group: Exclude<ShortcutGroupId, "dates">;
	readonly action: ShortcutActionId;
	readonly mac: readonly string[];
	readonly other: readonly string[];
	readonly when: ShortcutNeed;
}

/** Rows of the keyboard shortcuts sheet (spec S2, `FLP_SHORTCUTS`), in sheet order. */
const SHORTCUTS: readonly ShortcutSpec[] = [
	{
		group: "run",
		action: "run",
		mac: ["⌘↵"],
		other: ["Ctrl+Enter"],
		when: "always",
	},
	{
		group: "run",
		action: "runLeave",
		mac: ["⇧⌘↵"],
		other: ["Ctrl+Shift+Enter"],
		when: "fields",
	},
	{
		group: "run",
		action: "stop",
		mac: ["⌘."],
		other: ["Ctrl+."],
		when: "always",
	},
	{
		group: "move",
		action: "enterNext",
		mac: ["↵"],
		other: ["Enter"],
		when: "fields",
	},
	{
		group: "move",
		action: "tabNext",
		mac: ["Tab", "⇧Tab"],
		other: ["Tab", "Shift+Tab"],
		when: "fields",
	},
	{ group: "move", action: "filter", mac: ["/"], other: ["/"], when: "many" },
	{
		group: "move",
		action: "runTabs",
		mac: ["←", "→"],
		other: ["←", "→"],
		when: "always",
	},
	{ group: "faster", action: "recent", mac: ["↓"], other: ["↓"], when: "text" },
	{
		group: "faster",
		action: "acceptSuggestion",
		mac: ["→", "End"],
		other: ["→", "End"],
		when: "text",
	},
	{
		group: "faster",
		action: "resetField",
		mac: ["⇧⌘⌫"],
		other: ["Ctrl+Shift+Backspace"],
		when: "fields",
	},
	{
		group: "faster",
		action: "chooseFiles",
		mac: ["⌘O"],
		other: ["Ctrl+O"],
		when: "file",
	},
	{
		group: "faster",
		action: "replaceFile",
		mac: ["Space"],
		other: ["Space"],
		when: "file",
	},
	{
		group: "faster",
		action: "removeFile",
		mac: ["⌫"],
		other: ["Backspace"],
		when: "file",
	},
	{
		group: "faster",
		action: "undo",
		mac: ["⌘Z"],
		other: ["Ctrl+Z"],
		when: "fields",
	},
	{
		group: "faster",
		action: "calendar",
		mac: ["⌥↓"],
		other: ["Alt+↓"],
		when: "date",
	},
	{
		group: "faster",
		action: "nudge",
		mac: ["↑", "↓", "⇧↑", "⇧↓"],
		other: ["↑", "↓", "Shift+↑", "Shift+↓"],
		when: "numbers",
	},
	{
		group: "faster",
		action: "presets",
		mac: ["⌘P"],
		other: ["Ctrl+P"],
		when: "presets",
	},
	{
		group: "faster",
		action: "savePreset",
		mac: ["⌘S"],
		other: ["Ctrl+S"],
		when: "presets",
	},
];

export interface ShortcutOptions {
	/** The date field's anchor (`dateAnchorOf`) the date examples read against; null reads against today. */
	readonly anchorIso: string | null;
	readonly todayIso: string;
	readonly mac: boolean;
	/** The shoulds that exist: S1 rows need `presets`, the S3 row `numbers`. */
	readonly built: { readonly presets: boolean; readonly numbers: boolean };
	readonly dateLocale: DateLocale;
}

/** Kinds of the fields and of object properties; a list of texts counts as `textList`. */
function kindsOf(fields: readonly WorkbenchField[]) {
	const kindOf = (field: WorkbenchField) =>
		field.kind === "chips" && field.itemKind === "text"
			? "textList"
			: field.kind;
	return new Set(
		fields.flatMap((field) => [kindOf(field), ...field.props.map(kindOf)]),
	);
}

function needsMet(
	fields: readonly WorkbenchField[],
	built: ShortcutOptions["built"],
) {
	const kinds: ReadonlySet<string> = kindsOf(fields);
	const has = (...wanted: (FieldKind | "textList")[]) =>
		wanted.some((kind) => kinds.has(kind));
	const met: Readonly<Record<ShortcutNeed, boolean>> = {
		always: true,
		fields: fields.length > 0,
		many: fields.length >= FORM_LIMITS.filterFromFields,
		text: has("text", "textList"),
		file: has("file", "files"),
		date: has("date"),
		numbers: built.numbers && has("number"),
		presets: built.presets && fields.length > 0,
	};
	return met;
}

/** "Typing dates": the examples in the viewer's order, each with the date it reads as against the anchor. */
function dateRows(options: ShortcutOptions) {
	const { dateLocale: locale, anchorIso, todayIso } = options;
	const sep = locale.sep;
	const dayMonth = locale.order === "dmy" ? `18${sep}9` : `9${sep}18`;
	const full =
		locale.order === "ymd" ? `2026${sep}${dayMonth}` : `${dayMonth}${sep}26`;
	const readsAs = (text: string) =>
		readDate(text, anchorIso, todayIso, locale)?.iso ?? "";
	const { today, yesterday, tomorrow } = locale.words;
	const rows: ShortcutRow[] = [
		{ action: "dateDay", keys: ["18"], example: readsAs("18") },
		{ action: "dateDayMonth", keys: [dayMonth], example: readsAs(dayMonth) },
		{ action: "dateFull", keys: [full], example: readsAs(full) },
		{ action: "dateWords", keys: [today[0], yesterday[0], tomorrow[0]] },
	];
	return rows;
}

/**
 * The shortcuts sheet's groups for one form (spec S2, `flpShortcuts`), listing only what applies: a form
 * without fields gets "Run" and "Move around"; "/" needs FORM_LIMITS.filterFromFields fields; the dates group
 * needs a date field. Keys are ⌘ chips on a Mac, Ctrl words elsewhere.
 */
export function shortcutGroups(
	fields: readonly WorkbenchField[],
	options: ShortcutOptions,
) {
	const met = needsMet(fields, options.built);
	const groups: { id: ShortcutGroupId; rows: ShortcutRow[] }[] = [];
	for (const spec of SHORTCUTS) {
		if (!met[spec.when]) continue;
		let group = groups.find((candidate) => candidate.id === spec.group);
		if (!group) {
			group = { id: spec.group, rows: [] };
			groups.push(group);
		}
		group.rows.push({
			action: spec.action,
			keys: options.mac ? spec.mac : spec.other,
		});
	}
	if (met.date) groups.push({ id: "dates", rows: dateRows(options) });
	return groups;
}
