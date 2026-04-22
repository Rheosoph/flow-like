import type { PhrasingContent, Root, Text } from "mdast";
import { SKIP, visit } from "unist-util-visit";

/**
 * Remark plugin that parses `||text||` inline spoiler syntax.
 * Converts occurrences to links with the `spoiler://` protocol so they can be
 * deserialized by Plate and rendered as inline spoiler elements.
 */
export function remarkInlineSpoiler() {
	return (tree: Root) => {
		visit(tree, "text", (node: Text, index, parent) => {
			if (!parent || index === undefined) return;

			const regex = /\|\|([^|]+)\|\|/g;
			const text = node.value;

			if (!regex.test(text)) return;
			regex.lastIndex = 0;

			const parts: PhrasingContent[] = [];
			let lastIndex = 0;
			let match: RegExpExecArray | null;

			while ((match = regex.exec(text)) !== null) {
				if (match.index > lastIndex) {
					parts.push({
						type: "text",
						value: text.slice(lastIndex, match.index),
					} as Text);
				}

				const spoilerText = match[1];
				parts.push({
					type: "link",
					url: `spoiler://${encodeURIComponent(spoilerText)}`,
					children: [{ type: "text", value: spoilerText } as Text],
				} as any);

				lastIndex = regex.lastIndex;
			}

			if (lastIndex < text.length) {
				parts.push({
					type: "text",
					value: text.slice(lastIndex),
				} as Text);
			}

			if (parts.length > 0) {
				parent.children.splice(index, 1, ...parts);
				return [SKIP, index + parts.length] as const;
			}
		});
	};
}
