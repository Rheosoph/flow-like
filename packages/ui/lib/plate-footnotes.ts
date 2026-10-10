import type { TElement, Value } from "platejs";

export const FOOTNOTE_REFERENCE = "footnoteReference";
export const FOOTNOTE_DEFINITION = "footnoteDefinition";
export type FootnoteElement = TElement & { identifier: string };

export function footnoteDomId(
	kind: "note" | "ref",
	scope: string,
	identifier: string,
): string {
	const encode = (value: string) =>
		Array.from(value)
			.map((char) => char.codePointAt(0)!.toString(16))
			.join("-") || "0";
	return `fn-${kind}-${encode(scope)}-${encode(identifier)}`;
}

export function nextFootnoteIdentifier(nodes: Value): string {
	const used = new Set<string>();
	const visit = (items: Value) => {
		for (const node of items) {
			if (typeof node.identifier === "string") used.add(node.identifier);
			if (Array.isArray(node.children)) visit(node.children as Value);
		}
	};
	visit(nodes);
	let next = 1;
	while (used.has(String(next))) next++;
	return String(next);
}

/** Repeated references share a definition but each link needs its own DOM ID. */
export function footnoteReferenceOccurrence(
	nodes: Value,
	target: FootnoteElement,
): number {
	let count = 0;
	let found = false;
	const visit = (items: Value) => {
		for (const node of items) {
			if (found) return;
			if (node === target) {
				found = true;
				return;
			}
			if (
				node.type === FOOTNOTE_REFERENCE &&
				node.identifier === target.identifier
			)
				count++;
			if (Array.isArray(node.children)) visit(node.children as Value);
		}
	};
	visit(nodes);
	return count;
}
