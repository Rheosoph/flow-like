const PLATE_JSON_PREFIX = "plate_json::";

interface PlateNode {
	readonly text?: string;
	readonly value?: string;
	readonly children?: ReadonlyArray<PlateNode>;
	readonly [key: string]: unknown;
}

function nodeText(node: PlateNode): string {
	if (typeof node.text === "string") return node.text;
	if (Array.isArray(node.children)) {
		return node.children.map(nodeText).join("");
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
