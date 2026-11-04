const FENCE = /^\s*```/;
/** Table rows and separators, horizontal rules. */
const SKIPPED_LINE = /^\s*(?:\||(?:[-*_]\s*){3,}$)/;
const MARKDOWN_SYNTAX: readonly (readonly [RegExp, string])[] = [
	[/!\[([^\]]*)\]\([^)]*\)/g, "$1"],
	[/\[([^\]]+)\]\([^)]*\)/g, "$1"],
	[/<[^>]+>/g, ""],
	[/^\s{0,3}(?:#{1,6}\s+|>\s?|[-*+]\s+|\d+[.)]\s+)/, ""],
	[/\*\*|__|~~|`/g, ""],
	[/\*(\S[^*]*)\*/g, "$1"],
	[/\s+/g, " "],
];

function plainLine(line: string) {
	let text = line;
	for (const [pattern, replacement] of MARKDOWN_SYNTAX)
		text = text.replace(pattern, replacement);
	return text.trim();
}

/** The readable lines of markdown as plain text, skipping code fences, tables and rules ("## Invoice extracted" → "Invoice extracted"). */
export function* plainMarkdownLines(text: string): Generator<string, void> {
	let fenced = false;
	for (const line of text.split(/\r?\n/)) {
		if (FENCE.test(line)) {
			fenced = !fenced;
			continue;
		}
		if (fenced || SKIPPED_LINE.test(line)) continue;
		const plain = plainLine(line);
		if (plain) yield plain;
	}
}
