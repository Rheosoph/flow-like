import type { ILogChip } from "./filter-model";
import { ALL_LEVELS, levelFromName } from "./log-format";

export interface IQueryToken {
	negated: boolean;
	key?: "node" | "level";
	value: string;
	quoted: boolean;
	/** A quote was opened and never closed. */
	open: boolean;
	raw: string;
}

export interface INodeRef {
	id: string;
	name: string;
	friendlyName: string;
}

const KEYS = ["node", "level"] as const;

function readValue(
	input: string,
	from: number,
): { value: string; quoted: boolean; open: boolean; end: number } {
	if (input[from] === '"') {
		const close = input.indexOf('"', from + 1);
		if (close < 0) {
			return {
				value: input.slice(from + 1),
				quoted: true,
				open: true,
				end: input.length,
			};
		}
		return {
			value: input.slice(from + 1, close),
			quoted: true,
			open: false,
			end: close + 1,
		};
	}
	let end = from;
	while (end < input.length && !/\s/.test(input[end])) end++;
	return { value: input.slice(from, end), quoted: false, open: false, end };
}

export function tokenize(input: string): IQueryToken[] {
	const tokens: IQueryToken[] = [];
	let i = 0;
	while (i < input.length) {
		if (/\s/.test(input[i])) {
			i++;
			continue;
		}
		const start = i;
		let negated = false;
		if (input[i] === "-" && i + 1 < input.length && !/\s/.test(input[i + 1])) {
			negated = true;
			i++;
		}
		let key: IQueryToken["key"];
		const rest = input.slice(i, i + 6).toLowerCase();
		for (const candidate of KEYS) {
			if (rest.startsWith(`${candidate}:`)) {
				key = candidate;
				i += candidate.length + 1;
				break;
			}
		}
		const { value, quoted, open, end } = readValue(input, i);
		i = Math.max(end, i + 1);
		tokens.push({
			negated,
			key,
			value,
			quoted,
			open,
			raw: input.slice(start, end),
		});
	}
	return tokens;
}

/** Exact, case-insensitive: the friendly name first, then the catalog name. */
export function resolveNode(
	name: string,
	nodes: readonly INodeRef[],
): string | undefined {
	const wanted = name.trim().toLowerCase();
	if (!wanted) return undefined;
	return (
		nodes.find((node) => node.friendlyName.toLowerCase() === wanted)?.id ??
		nodes.find((node) => node.name.toLowerCase() === wanted)?.id
	);
}

/** `error` or `warn,error`; undefined when any part is not a level. */
export function parseLevels(value: string): number[] | undefined {
	const parts = value
		.split(",")
		.map((part) => part.trim())
		.filter(Boolean);
	if (parts.length === 0) return undefined;
	const levels: number[] = [];
	for (const part of parts) {
		const level = levelFromName(part);
		if (level === undefined) return undefined;
		levels.push(level);
	}
	return levels;
}

export interface IParsedInput {
	chips: ILogChip[];
	/** Levels a `level:` token asked for exclusively. */
	onlyLevels: number[];
	/** Levels a `-level:` token hid. */
	hideLevels: number[];
}

export interface IParseOptions {
	/**
	 * Committing turns what does not resolve into text. A live preview skips
	 * it instead, so a half-typed `node:Ups` never filters for that text.
	 */
	commit: boolean;
}

export function parseQueryInput(
	input: string,
	nodes: readonly INodeRef[],
	options: IParseOptions,
): IParsedInput {
	const out: IParsedInput = { chips: [], onlyLevels: [], hideLevels: [] };
	const asText = (token: IQueryToken) => {
		const text = token.negated ? token.raw.slice(1) : token.raw;
		if (text)
			out.chips.push({ kind: "text", value: text, negated: token.negated });
	};
	for (const token of tokenize(input)) {
		if (token.key === "node") {
			const id = token.value ? resolveNode(token.value, nodes) : undefined;
			if (id)
				out.chips.push({ kind: "node", value: id, negated: token.negated });
			else if (options.commit && token.value) asText(token);
			continue;
		}
		if (token.key === "level") {
			const levels = parseLevels(token.value);
			if (levels) {
				(token.negated ? out.hideLevels : out.onlyLevels).push(...levels);
			} else if (options.commit && token.value) asText(token);
			continue;
		}
		if (!token.value || (!token.quoted && token.value === "-")) continue;
		out.chips.push({
			kind: "text",
			value: token.value,
			negated: token.negated,
		});
	}
	return out;
}

/** The level set a parsed input leaves hidden, starting from `levelsOff`. */
export function levelsAfter(
	levelsOff: readonly number[],
	parsed: Pick<IParsedInput, "onlyLevels" | "hideLevels">,
): number[] {
	const off = new Set(
		parsed.onlyLevels.length > 0
			? ALL_LEVELS.filter((level) => !parsed.onlyLevels.includes(level))
			: levelsOff,
	);
	for (const level of parsed.hideLevels) off.add(level);
	return [...off].sort((a, b) => a - b);
}

/** The token being typed at the end of the input, for autocomplete. */
export function trailingToken(input: string): {
	token?: IQueryToken;
	prefix: string;
} {
	if (!input || /\s$/.test(input)) {
		const open = tokenize(input).at(-1);
		if (open?.open)
			return {
				token: open,
				prefix: input.slice(0, input.length - open.raw.length),
			};
		return { prefix: input };
	}
	const tokens = tokenize(input);
	const token = tokens.at(-1);
	if (!token) return { prefix: input };
	return { token, prefix: input.slice(0, input.length - token.raw.length) };
}

export function quoteValue(value: string): string {
	return /[\s"]/.test(value) ? `"${value.replace(/"/g, "")}"` : value;
}
