import type { INode } from "./schema/flow/node";

const DOCS_BASE_URL = "https://docs.flow-like.com";

function safeSegment(value?: string | null): string {
	if (!value) return "node";

	let out = "";
	let lastWasDash = false;

	for (const ch of value.trim()) {
		const code = ch.charCodeAt(0);
		const isAsciiDigit = code >= 48 && code <= 57;
		const isAsciiUpper = code >= 65 && code <= 90;
		const isAsciiLower = code >= 97 && code <= 122;

		if (isAsciiDigit || isAsciiUpper || isAsciiLower) {
			out += ch.toLowerCase();
			lastWasDash = false;
		} else if (ch === " " || ch === "_" || ch === "-" || ch === ".") {
			if (!lastWasDash && out.length > 0) {
				out += "-";
				lastWasDash = true;
			}
		} else if (!lastWasDash && out.length > 0) {
			out += "-";
			lastWasDash = true;
		}
	}

	while (out.endsWith("-")) {
		out = out.slice(0, -1);
	}

	return out || "node";
}

function categorySegments(category?: string | null): string[] {
	if (!category) return ["Uncategorized"];

	const segments = category
		.split("/")
		.map((segment) => segment.trim())
		.filter(Boolean);

	return segments.length > 0 ? segments : ["Uncategorized"];
}

function safeDocsUrl(value?: string | null): string | null {
	const docsUrl = value?.trim();
	if (!docsUrl) return null;

	try {
		const url = new URL(docsUrl);
		return url.protocol === "http:" || url.protocol === "https:"
			? url.toString()
			: null;
	} catch {
		return null;
	}
}

export function buildNodeDocsUrl(node: INode): string {
	const docsUrl = safeDocsUrl(node.docs);
	if (docsUrl) {
		return docsUrl;
	}

	const categoryPath = categorySegments(node.category)
		.map(safeSegment)
		.join("/");
	const nodeSlug = safeSegment(node.name);

	return `${DOCS_BASE_URL}/nodes/${categoryPath}/${nodeSlug}/`;
}
