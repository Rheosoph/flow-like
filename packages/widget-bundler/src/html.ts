export function escapeHtmlAttr(value: string): string {
	return value
		.replace(/&/g, "&amp;")
		.replace(/"/g, "&quot;")
		.replace(/</g, "&lt;")
		.replace(/>/g, "&gt;");
}

/**
 * Insert a fragment at the very start of `<head>`, creating the element when
 * the document has none.
 */
export function insertAtHeadStart(html: string, fragment: string): string {
	const head = /<head\b[^>]*>/i.exec(html);
	if (head) {
		const end = head.index + head[0].length;
		return `${html.slice(0, end)}${fragment}${html.slice(end)}`;
	}
	const htmlTag = /<html\b[^>]*>/i.exec(html);
	if (htmlTag) {
		const end = htmlTag.index + htmlTag[0].length;
		return `${html.slice(0, end)}<head>${fragment}</head>${html.slice(end)}`;
	}
	const doctype = /^\s*<!doctype\b[^>]*>/i.exec(html);
	if (doctype) {
		const end = doctype.index + doctype[0].length;
		return `${html.slice(0, end)}<head>${fragment}</head>${html.slice(end)}`;
	}
	return `<head>${fragment}</head>${html}`;
}
