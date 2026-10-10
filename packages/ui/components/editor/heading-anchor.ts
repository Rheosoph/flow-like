import type { SlateEditor, TElement } from "platejs";

export function headingAnchor(
	editor: SlateEditor,
	node: TElement,
	path?: number[],
): string {
	const location =
		path ??
		[
			...editor.api.nodes<TElement>({
				at: [],
				match: (candidate) => candidate === node,
			}),
		][0]?.[1];
	return `heading-${(location ?? []).join("-")}`;
}
