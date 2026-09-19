import type { ColumnKind } from "../settings/data-studio/query-workbench/column-types";

export const HOME_DATA_EMPTY_LABEL = "No value";

export function homeDataText(value: unknown): string {
	if (value === null || value === undefined) return HOME_DATA_EMPTY_LABEL;
	return typeof value === "object" ? JSON.stringify(value) : String(value);
}

const ACRONYMS = new Set([
	"api",
	"csv",
	"html",
	"id",
	"ip",
	"json",
	"pdf",
	"sku",
	"sql",
	"uri",
	"url",
	"uuid",
]);
/** Trailing words a reader does not need once the value itself shows the kind. */
const IMPLIED_SUFFIXES: Partial<Record<ColumnKind, ReadonlySet<string>>> = {
	temporal: new Set(["at", "on"]),
	user: new Set(["id", "uid", "uuid", "sub", "ref", "key"]),
};

/**
 * A column name as a person would write it: `first_seen_at` → `First seen`,
 * `owner_id` → `Owner`, `feedsChecked` → `Feeds checked`. Letters outside ASCII
 * survive, so non-English names are only re-spaced, never cut apart.
 */
export function homeDataFieldLabel(name: string, kind?: ColumnKind): string {
	const words = name
		.replace(/([\p{Ll}\p{N}])(\p{Lu})/gu, "$1 $2")
		.replace(/(\p{Lu})(\p{Lu}(?!s(?!\p{Ll}))\p{Ll})/gu, "$1 $2")
		.split(/[\s_\-.]+/u)
		.filter(Boolean);
	// `LAST_UPDATED` is shouting; a lone `MRR` or `NPS` is an acronym.
	const shouting = name === name.toUpperCase() && words.length > 1;
	const implied = kind ? IMPLIED_SUFFIXES[kind] : undefined;
	while (
		implied &&
		words.length > 1 &&
		implied.has(words[words.length - 1].toLowerCase())
	)
		words.pop();
	if (!words.length) return name;
	return words
		.map((word, index) => {
			const lower = word.toLowerCase();
			if (ACRONYMS.has(lower)) return lower.toUpperCase();
			if (lower.endsWith("s") && ACRONYMS.has(lower.slice(0, -1)))
				return `${lower.slice(0, -1).toUpperCase()}s`;
			if (!shouting && word.length > 1 && word === word.toUpperCase())
				return word;
			return index === 0
				? `${lower.charAt(0).toUpperCase()}${lower.slice(1)}`
				: lower;
		})
		.join(" ");
}

/** The same label read mid-sentence, as in "by first seen" or "Sum of amount". */
export function homeDataInlineFieldLabel(
	name: string,
	kind?: ColumnKind,
): string {
	const label = homeDataFieldLabel(name, kind);
	const [first = ""] = label.split(" ");
	const keepsCase =
		/\p{Lu}/u.test(first.slice(1)) ||
		(first.length > 1 && !/\p{Ll}/u.test(first));
	return keepsCase
		? label
		: `${label.charAt(0).toLowerCase()}${label.slice(1)}`;
}
