const PLATE_JSON_PREFIX = "plate_json::";

interface PlateNode {
	readonly type?: string;
	readonly text?: string;
	readonly value?: string;
	readonly children?: ReadonlyArray<PlateNode>;
	readonly [key: string]: unknown;
}

const INLINE_TYPES = new Set([
	"a",
	"date",
	"emoji_input",
	"focus_node",
	"footnoteReference",
	"inline_equation",
	"inline_spoiler",
	"mention",
	"mention_input",
	"slash_input",
	"user_mention",
]);

/**
 * Quotes, callouts, columns and table cells hold whole blocks — since Plate 53
 * a quote's text sits in paragraphs — and those read as separate lines.
 */
function holdsBlocks(children: ReadonlyArray<PlateNode>): boolean {
	return (
		children.every((child) => typeof child.text !== "string") &&
		children.some((child) => !INLINE_TYPES.has(child.type ?? ""))
	);
}

function nodeText(node: PlateNode): string {
	if (typeof node.text === "string") return node.text;
	if (Array.isArray(node.children)) {
		const texts = node.children.map(nodeText);
		return holdsBlocks(node.children)
			? texts.filter((text) => text.length > 0).join("\n")
			: texts.join("");
	}
	return typeof node.value === "string" ? node.value : "";
}

/**
 * The plain text of a rich-text field, for places that list content rather than
 * render it — a comment row, a search result, a tooltip.
 *
 * Rich fields are stored either as markdown or as `plate_json::` followed by the
 * editor's node array; anything that lists them raw shows the serialized JSON,
 * which is what the comments sidebar did.
 */
export function plainTextFromRichContent(content: string): string {
	if (!content.startsWith(PLATE_JSON_PREFIX)) return content;
	try {
		const parsed: unknown = JSON.parse(content.slice(PLATE_JSON_PREFIX.length));
		if (!Array.isArray(parsed)) return "";
		return (parsed as PlateNode[])
			.map(nodeText)
			.filter((line) => line.length > 0)
			.join("\n")
			.trim();
	} catch {
		return "";
	}
}
