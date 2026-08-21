export function escapeHtmlAttr(value: string): string {
	return value
		.replace(/&/g, "&amp;")
		.replace(/"/g, "&quot;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;");
}

const WORD_CHAR = /\w/;

export interface TagSpan {
	start: number;
	/** Offset just past the tag's `>` */
	end: number;
}

/**
 * Linear-time equivalent of `/<name\b[^>]*>/i`: the next start tag for the
 * (lowercase) `name` at or after `from`. The regex form is quadratic in the
 * document length because every `<` is a candidate start for an `[^>]*` scan.
 */
export function findTag(html: string, name: string, from = 0): TagSpan | null {
	let cursor = from;
	for (;;) {
		const start = html.indexOf("<", cursor);
		if (start === -1) return null;
		const nameEnd = start + 1 + name.length;
		if (
			html.slice(start + 1, nameEnd).toLowerCase() !== name ||
			WORD_CHAR.test(html[nameEnd] ?? "")
		) {
			cursor = start + 1;
			continue;
		}
		const close = html.indexOf(">", nameEnd);
		return close === -1 ? null : { start, end: close + 1 };
	}
}

/**
 * Insert a fragment at the very start of `<head>`, creating the element when
 * the document has none.
 */
export function insertAtHeadStart(html: string, fragment: string): string {
	const head = findTag(html, "head");
	if (head) {
		return `${html.slice(0, head.end)}${fragment}${html.slice(head.end)}`;
	}
	const htmlTag = findTag(html, "html");
	if (htmlTag) {
		const { end } = htmlTag;
		return `${html.slice(0, end)}<head>${fragment}</head>${html.slice(end)}`;
	}
	const doctype = /^\s*<!doctype\b[^>]*>/i.exec(html);
	if (doctype) {
		const end = doctype.index + doctype[0].length;
		return `${html.slice(0, end)}<head>${fragment}</head>${html.slice(end)}`;
	}
	return `<head>${fragment}</head>${html}`;
}
