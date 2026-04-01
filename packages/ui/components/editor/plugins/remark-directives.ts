/**
 * Preprocesses raw markdown to convert `:::type` directive blocks into
 * fenced code blocks with a `directive-{type}` language tag.
 *
 * This runs BEFORE the markdown parser so it handles all content correctly,
 * including inline code, blank lines, and nested code blocks within directives.
 *
 * Supported directive types:
 * - `:::info`, `:::warning`, `:::error`, `:::success`, `:::tip` → admonition blocks
 * - `:::spoiler [label]` → collapsible spoiler blocks
 *
 * Output: fenced code blocks that Plate routes to custom renderers.
 */

const DIRECTIVE_TYPES = [
	"info",
	"warning",
	"error",
	"success",
	"tip",
	"spoiler",
];

export function preprocessDirectiveBlocks(markdown: string): string {
	const lines = markdown.split("\n");
	const output: string[] = [];
	let i = 0;

	while (i < lines.length) {
		const line = lines[i];
		const match = line.match(/^:::(\w+)(?:\s+(.+))?$/);

		if (match) {
			const type = match[1].toLowerCase();
			const title = (match[2] || "").trim();

			if (DIRECTIVE_TYPES.includes(type)) {
				const contentLines: string[] = [];
				let j = i + 1;
				let found = false;

				while (j < lines.length) {
					if (lines[j].trim() === ":::") {
						found = true;
						break;
					}
					contentLines.push(lines[j]);
					j++;
				}

				if (found) {
					const lang = `directive-${type}`;
					const innerContent = contentLines.join("\n");

					// Find safe fence length (must exceed any backtick sequence in content)
					let maxBackticks = 2;
					const backticksMatches = innerContent.match(/`{3,}/g);
					if (backticksMatches) {
						for (const m of backticksMatches) {
							maxBackticks = Math.max(maxBackticks, m.length);
						}
					}
					const fence = "`".repeat(Math.max(3, maxBackticks + 1));

					const content = title
						? `${title}\n---\n${innerContent}`
						: innerContent;

					output.push(`${fence}${lang}`);
					output.push(content);
					output.push(fence);

					i = j + 1;
					continue;
				}
			}
		}

		output.push(line);
		i++;
	}

	return output.join("\n");
}
