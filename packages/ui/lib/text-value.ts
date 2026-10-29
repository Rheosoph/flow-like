/** Strings past this are not parsed; JSON.parse on them would stall the dialog. */
export const MAX_JSON_PARSE_LENGTH = 2_000_000;

export type JsonContainer = unknown[] | Record<string, unknown>;

export type TextReading =
	| { kind: "json"; value: JsonContainer }
	| { kind: "markup" }
	| { kind: "text" };

const CLOSING_BRACKET: Record<string, string> = { "{": "}", "[": "]" };

function isBracketed(text: string) {
	const close = CLOSING_BRACKET[text[0]];
	return close !== undefined && text.endsWith(close);
}

/** The JSON object or array a string holds, when all of it is one. */
export function parseJsonContainer(text: string): JsonContainer | undefined {
	const trimmed = text.trim();
	if (trimmed.length > MAX_JSON_PARSE_LENGTH || !isBracketed(trimmed)) {
		return undefined;
	}
	try {
		return JSON.parse(trimmed) as JsonContainer;
	} catch {
		return undefined;
	}
}

/**
 * XML and HTML. Markdown would read `<ram:ID>` as an autolink and drop the
 * rest of the tags, so these stay verbatim.
 */
export function looksLikeMarkup(text: string): boolean {
	const trimmed = text.trim();
	return /^<[?!a-zA-Z]/.test(trimmed) && trimmed.endsWith(">");
}

/** What a stored string holds, so the detail view can pick a reading that keeps it intact. */
export function readTextValue(text: string): TextReading {
	const json = parseJsonContainer(text);
	if (json) return { kind: "json", value: json };
	if (looksLikeMarkup(text)) return { kind: "markup" };
	return { kind: "text" };
}
