/**
 * Component types a chat reply renders (`manifest.chat.components`, drift-tested). Inline so every
 * markdown surface can recognise an a2ui block without loading the component manifest.
 */
export const CHAT_A2UI_COMPONENT_TYPES: ReadonlySet<string> = new Set([
	"column",
	"row",
	"grid",
	"stack",
	"box",
	"center",
	"spacer",
	"scrollArea",
	"aspectRatio",
	"text",
	"markdown",
	"image",
	"icon",
	"badge",
	"avatar",
	"progress",
	"divider",
	"skeleton",
	"card",
	"tabs",
	"accordion",
	"tooltip",
	"button",
	"link",
	"textField",
	"select",
	"slider",
	"checkbox",
	"switch",
	"radioGroup",
	"dateTimeInput",
	"table",
	"nivoChart",
]);

/** Tags models put on JSON when they forget `a2ui`, plus an untagged fence. */
const JSON_FENCE_LANGUAGES: ReadonlySet<string> = new Set([
	"",
	"json",
	"jsonl",
	"ndjson",
	"jsonc",
	"json5",
]);

const SNIFF_CHARS = 600;
const TYPE_FIELD = /"type"\s*:\s*"([A-Za-z0-9]+)"/;
const STRING_ID_FIELD = /"id"\s*:\s*"/;

/**
 * Whether a block's opening reads as a2ui components: a JSON object (or a list of them) with a
 * string `id` whose first `type` is a chat component. Generic JSON with `id`/`type` keys rarely
 * names one of these camelCase component types.
 */
export function looksLikeA2UIComponents(content: string): boolean {
	const head = content.trimStart().slice(0, SNIFF_CHARS);
	if (!head.startsWith("{") && !head.startsWith("[")) return false;
	const type = TYPE_FIELD.exec(head)?.[1];
	return (
		type !== undefined &&
		CHAT_A2UI_COMPONENT_TYPES.has(type) &&
		STRING_ID_FIELD.test(head)
	);
}

/**
 * A fence renders as an a2ui surface when it is tagged `a2ui`, or — inside a chat reply — when
 * it is untagged or tagged as JSON and its first object is a chat component, so a model that
 * forgets the tag still shows its interface.
 */
export function isA2UIFence(
	lang: string | undefined,
	content: string,
	inChat: boolean,
): boolean {
	const language = lang?.trim().toLowerCase() ?? "";
	if (language === "a2ui") return true;
	if (!inChat || !JSON_FENCE_LANGUAGES.has(language)) return false;
	return looksLikeA2UIComponents(content);
}
