const URL_START = /^(?:[a-z][a-z\d+.-]*:\/\/|mailto:|tel:|sandbox:|www\.)/i;
const URL_PREFIXES = ["https://", "http://", "www.", "mailto:", "sandbox:"];
const ENTITIES: Record<string, string> = {
	amp: "&",
	quot: '"',
	apos: "'",
	lt: "<",
	gt: ">",
	nbsp: " ",
};

/** Find a matching bracket without mistaking escaped or nested brackets for its end. */
function closingBracket(
	text: string,
	start: number,
	open: string,
	close: string,
) {
	let depth = 1;
	let quote: string | undefined;
	for (let at = start + 1; at < text.length; at++) {
		if (text[at] === "\\") at++;
		else if (quote) {
			if (text[at] === quote) quote = undefined;
		} else if (open === "(" && /["']/.test(text[at]) && /\s/.test(text[at - 1]))
			quote = text[at];
		else if (text[at] === open) depth++;
		else if (text[at] === close && --depth === 0) return at;
	}
	return -1;
}

function inlineText(text: string, streaming: boolean): string {
	let result = "";
	for (let at = 0; at < text.length; ) {
		const rest = text.slice(at);
		if (
			streaming &&
			text[at] === "(" &&
			URL_PREFIXES.some((prefix) =>
				prefix.startsWith(rest.slice(1).toLowerCase()),
			)
		)
			break;
		if (text[at] === "(" && URL_START.test(text.slice(at + 1))) {
			const end = closingBracket(text, at, "(", ")");
			if (end < 0 && !/\s/.test(rest)) break;
			if (end >= 0 && !/\s/.test(text.slice(at + 1, end))) {
				at = end + 1;
				continue;
			}
		}
		if (URL_START.test(rest)) {
			const url = /^[^\s<>]+/.exec(rest)?.[0] ?? rest;
			// Keep sentence punctuation only once the URL can no longer grow.
			if (!streaming || at + url.length < text.length)
				result += /[.!?,;:]+$/.exec(url)?.[0] ?? "";
			at += url.length;
			continue;
		}
		if (
			streaming &&
			(at === 0 || /\s/.test(text[at - 1])) &&
			URL_PREFIXES.some((prefix) => prefix.startsWith(rest.toLowerCase()))
		)
			break;
		if (text[at] === "\\") {
			if (at + 1 === text.length && streaming) break;
			result += text[at + 1] ?? "";
			at += 2;
			continue;
		}
		if (text[at] === "`") {
			const marker = /^`+/.exec(rest)?.[0] ?? "`";
			const end = text.indexOf(marker, at + marker.length);
			if (end < 0) break;
			result += inlineText(text.slice(at + marker.length, end), false);
			at = end + marker.length;
			continue;
		}
		const image = text[at] === "!" && text[at + 1] === "[";
		if (image || text[at] === "[") {
			const start = at + Number(image);
			const end = closingBracket(text, start, "[", "]");
			if (end < 0) break;
			let after = end + 1;
			if (after === text.length && streaming) break;
			if (text[after] === "(" || text[after] === "[") {
				const close = text[after] === "(" ? ")" : "]";
				const targetEnd = closingBracket(text, after, text[after], close);
				if (targetEnd < 0) break;
				after = targetEnd + 1;
			}
			if (!image && text[start + 1] !== "^")
				result += inlineText(text.slice(start + 1, end), false);
			at = after;
			continue;
		}
		if (text[at] === "<" && at + 1 === text.length && streaming) break;
		if (text[at] === "<" && /[a-z!/?]/i.test(text[at + 1] ?? "")) {
			const end = text.indexOf(">", at + 1);
			if (end < 0) break;
			const hidden = /^<(script|style|a2ui)\b/i.exec(rest);
			if (hidden && !/\/\s*>$/.test(text.slice(at, end + 1))) {
				const close = new RegExp(`</${hidden[1]}\\s*>`, "i").exec(
					text.slice(end + 1),
				);
				if (!close) break;
				at = end + 1 + close.index + close[0].length;
			} else at = end + 1;
			continue;
		}
		if (text[at] === "&") {
			const entity = /^&(#x[\da-f]+|#\d+|[a-z]+);/i.exec(rest);
			if (entity) {
				const key = entity[1];
				const value = key.startsWith("#")
					? Number.parseInt(
							key.slice(/^#x/i.test(key) ? 2 : 1),
							/^#x/i.test(key) ? 16 : 10,
						)
					: undefined;
				result +=
					value !== undefined && value > 0 && value <= 0x10ffff
						? String.fromCodePoint(value)
						: (ENTITIES[key] ?? entity[0]);
				at += entity[0].length;
				continue;
			}
			if (streaming && /^&(?:#x?[\da-f]*|[a-z]*)$/i.test(rest)) break;
		}
		if (rest.startsWith("__")) {
			at += /^_+/.exec(rest)?.[0].length ?? 2;
			continue;
		}
		if (
			/[*~]/.test(text[at]) ||
			(text[at] === "_" && !/\w_\w/.test(text.slice(at - 1, at + 2)))
		) {
			at++;
			continue;
		}
		result += text[at++];
	}
	return result;
}

/**
 * Text for call audio. Parse the whole reply before splitting speech so a chunk cannot lose
 * its enclosing code fence or link. Streaming withholds unfinished inline constructs, even
 * when a tool pause flushes the pending prose.
 */
export function callSpeechText(
	text: string,
	{ streaming = false }: { streaming?: boolean } = {},
): string {
	const lines = text.split(/\r?\n/);
	const prose: string[] = [];
	let fence: string | undefined;
	for (let index = 0; index < lines.length; index++) {
		const line = lines[index].replace(/^\s*(?:>\s*)+/, "");
		const marker = /^\s*(?:[-*+]\s+|\d+[.)]\s+)?(`{3,}|~{3,})(.*)$/.exec(line);
		if (fence) {
			if (
				marker &&
				marker[1][0] === fence[0] &&
				marker[1].length >= fence.length &&
				!marker[2].trim()
			)
				fence = undefined;
			continue;
		}
		if (marker) {
			fence = marker[1];
			continue;
		}
		if (/^\s*(?:\||(?:[-*_]\s*){3,}$|\[[^\]]+\]:|:::)/.test(line)) continue;
		if (/^(?: {4}|\t)/.test(line) && !/^\s*(?:[-*+]|\d+[.)])\s/.test(line))
			continue;
		if (
			streaming &&
			index === lines.length - 1 &&
			/^\s*(?:[`~]+|\d{1,9}[.)]?)$/.test(line)
		)
			continue;
		prose.push(
			line
				.replace(/^\s*(?:#{1,6}(?:\s+|$)|[-*+]\s+|\d+[.)]\s+)/, "")
				.replace(/^\[[ xX]\]\s+/, ""),
		);
	}
	return inlineText(prose.join("\n"), streaming)
		.split("\n")
		.map((line) =>
			line
				.replace(/\s+/g, " ")
				.replace(/ +([.,!?;:])/g, "$1")
				.trim(),
		)
		.filter((line) => /[\p{L}\p{N}\p{S}]/u.test(line))
		.join("\n");
}
