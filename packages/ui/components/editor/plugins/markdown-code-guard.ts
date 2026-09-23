import { type DeserializeMdOptions, deserializeMd } from "@platejs/markdown";
import type { SlateEditor, Value } from "platejs";

// The markdown parser rewrites every `<tag attr>` in its input to JSX before
// parsing (`class` → `className`, attributes quoted, void tags self-closed,
// HTML comments → JSX comments). That is meant for markup, but it runs on the
// raw string, so it also rewrote code samples and backslash-escaped `\<` text.
// Their tag openers are swapped for a private-use character the rewrite cannot
// match and swapped back in the parsed nodes.
const JSX_GUARD = "\uE000";
const TAG_OPENER = /<(?=[a-zA-Z0-9]|!--)/g;
const ESCAPED_TAG_OPENER = /(?<!\\)((?:\\\\)*)\\<(?=[a-zA-Z0-9]|!--)/g;
const FENCE_LINE =
	/^((?:[ \t]*>)*[ \t]*(?:(?:[-*+]|\d{1,9}[.)])[ \t]+)*)(`{3,}|~{3,})(.*)$/;
const CODE_SPAN = /(?<![`\\])(`+)(?!`)([\s\S]*?[^`])\1(?!`)/g;

function guardProse(text: string): string {
	let guarded = "";
	let last = 0;
	for (const span of text.matchAll(CODE_SPAN)) {
		guarded += text
			.slice(last, span.index)
			.replace(ESCAPED_TAG_OPENER, `$1${JSX_GUARD}`);
		guarded += span[0].replace(TAG_OPENER, JSX_GUARD);
		last = span.index + span[0].length;
	}
	return (
		guarded + text.slice(last).replace(ESCAPED_TAG_OPENER, `$1${JSX_GUARD}`)
	);
}

export function guardCodeFromJsx(markdown: string): string {
	if (!markdown.includes("<") || markdown.includes(JSX_GUARD)) return markdown;

	const output: string[] = [];
	let prose: string[] = [];
	let fence: string | null = null;
	const flushProse = () => {
		if (prose.length > 0) output.push(guardProse(prose.join("\n")));
		prose = [];
	};

	for (const line of markdown.split("\n")) {
		const marker = FENCE_LINE.exec(line);
		if (fence) {
			const closes =
				marker !== null &&
				marker[2][0] === fence[0] &&
				marker[2].length >= fence.length &&
				marker[3].trim() === "";
			if (closes) fence = null;
			output.push(closes ? line : line.replace(TAG_OPENER, JSX_GUARD));
			continue;
		}
		if (marker && !(marker[2][0] === "`" && marker[3].includes("`"))) {
			flushProse();
			fence = marker[2];
			output.push(line);
			continue;
		}
		if (line.trim() === "") {
			flushProse();
			output.push(line);
			continue;
		}
		prose.push(line);
	}
	flushProse();
	return output.join("\n");
}

function unguard(value: unknown): unknown {
	if (typeof value === "string") return value.replaceAll(JSX_GUARD, "<");
	if (Array.isArray(value)) return value.map(unguard);
	if (value === null || typeof value !== "object") return value;
	return Object.fromEntries(
		Object.entries(value).map(([key, entry]) => [key, unguard(entry)]),
	);
}

/** Parses `markdown` with the tag openers in its code guarded from the JSX rewrite. */
export function withCodeGuard<T>(
	markdown: string,
	parse: (guarded: string) => T,
): T {
	const guarded = guardCodeFromJsx(markdown);
	const parsed = parse(guarded);
	return guarded === markdown ? parsed : (unguard(parsed) as T);
}

/** `deserializeMd` with code kept verbatim; without MDX there is no rewrite to guard against. */
export function deserializeMdKeepingCode(
	editor: SlateEditor,
	markdown: string,
	options?: Omit<DeserializeMdOptions, "editor">,
): Value {
	if (options?.withoutMdx) return deserializeMd(editor, markdown, options);
	return withCodeGuard(markdown, (guarded) =>
		deserializeMd(editor, guarded, options),
	);
}
