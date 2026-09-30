import { parseFlowScriptAnchors } from "../flowscript/flowscript-anchors";

export interface IFlowScriptProjection {
	/** The source without anchor comments; line numbers are unchanged. */
	text: string;
	/** Board entity owning each line, index = line - 1. */
	owners: (string | undefined)[];
	/** First line (1-based) each entity owns. */
	firstLine: Map<string, number>;
}

/**
 * Anchored FlowScript is the only way to tie a rendered line back to a node, but the
 * `//@n:<id>` comments are noise in a diff. Strip them and keep the mapping; a closing
 * `}` (or `} else {`) belongs to the statement that opened its block.
 */
export function projectFlowScript(source: string): IFlowScriptProjection {
	const index = parseFlowScriptAnchors(source);
	const lines = source.split("\n");
	const owners: (string | undefined)[] = [];
	const firstLine = new Map<string, number>();
	const blocks: (string | undefined)[] = [];

	const text = lines
		.map((line, i) => {
			const anchor = index.byLine.get(i + 1);
			const visible = anchor
				? line.slice(0, anchor.column - 1).trimEnd()
				: line;
			const trimmed = visible.trim();
			let owner = anchor?.id;
			if (trimmed.startsWith("}")) {
				const opener = blocks.pop();
				owner ??= opener;
				if (trimmed.endsWith("{")) blocks.push(owner);
			} else if (trimmed.endsWith("{")) {
				blocks.push(owner);
			}
			owners.push(owner);
			if (owner && !firstLine.has(owner)) firstLine.set(owner, i + 1);
			return visible;
		})
		.join("\n");

	return { text, owners, firstLine };
}
