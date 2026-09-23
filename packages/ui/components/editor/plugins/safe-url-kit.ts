import {
	type Descendant,
	ElementApi,
	type SlateEditor,
	type TElement,
	TextApi,
	createSlatePlugin,
} from "platejs";

const SCRIPT_URL_PROTOCOLS = new Set(["javascript:", "vbscript:"]);

/**
 * False for `javascript:`/`vbscript:` URLs and for `data:` URLs that are not
 * images. The scheme comes from URL parsing, so `java\tscript:` is caught.
 */
export function isSafeEditorUrl(url: unknown) {
	if (typeof url !== "string") return true;
	let protocol: string;
	try {
		protocol = new URL(url, "https://import.invalid/").protocol;
	} catch {
		return true;
	}
	if (protocol === "data:") return /^\s*data:image\//i.test(url);
	return !SCRIPT_URL_PROTOCOLS.has(protocol);
}

const unwrapsUnsafeUrl = (editor: SlateEditor, node: TElement) =>
	editor.api.isInline(node) && !editor.api.isVoid(node);

/** Joins the text an unwrapped link leaves next to its siblings, as Slate's normalizer would. */
function mergeAdjacentTexts(nodes: Descendant[]): Descendant[] {
	const merged: Descendant[] = [];
	for (const node of nodes) {
		const previous = merged.at(-1);
		if (
			previous &&
			TextApi.isText(previous) &&
			TextApi.isText(node) &&
			TextApi.equals(previous, node, { loose: true })
		)
			merged[merged.length - 1] = {
				...previous,
				text: previous.text + node.text,
			};
		else merged.push(node);
	}
	return merged;
}

/** Unwraps inline elements and drops blocks and voids whose `url` would run script. */
export function withoutUnsafeUrls(
	editor: SlateEditor,
	nodes: Descendant[],
): Descendant[] {
	let changed = false;
	let unwrapped = false;
	const result = nodes.flatMap((node) => {
		if (!ElementApi.isElement(node)) return [node];
		const children = withoutUnsafeUrls(editor, node.children);
		const safe = isSafeEditorUrl(node.url);
		if (safe && children === node.children) return [node];
		changed = true;
		if (safe) return [{ ...node, children }];
		if (!unwrapsUnsafeUrl(editor, node)) return [];
		unwrapped = true;
		return children;
	});
	if (!changed) return nodes;
	return unwrapped ? mergeAdjacentTexts(result) : result;
}

/** Applies the same rule to every node an edit touches: paste, import, AI output, toolbar inserts. */
export const SafeUrlPlugin = createSlatePlugin({
	key: "safeUrl",
}).overrideEditor(({ editor, tf: { normalizeNode } }) => ({
	transforms: {
		normalizeNode(entry, options) {
			const [node, path] = entry;
			if (ElementApi.isElement(node) && !isSafeEditorUrl(node.url)) {
				if (unwrapsUnsafeUrl(editor, node)) editor.tf.unwrapNodes({ at: path });
				else editor.tf.removeNodes({ at: path });
				return;
			}
			normalizeNode(entry, options);
		},
	},
}));

export const SafeUrlKit = [SafeUrlPlugin];
