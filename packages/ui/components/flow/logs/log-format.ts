import type { ILog, ISystemTime } from "../../../lib/schema/flow/log";

export const LEVEL_COUNT = 5;
export const ALL_LEVELS: readonly number[] = [0, 1, 2, 3, 4];
export const LEVEL_NAMES = ["debug", "info", "warn", "error", "fatal"] as const;
export const LEVEL_LABELS = [
	"DEBUG",
	"INFO",
	"WARN",
	"ERROR",
	"FATAL",
] as const;
export const ERROR_LEVEL = 3;

const LEVEL_BY_NAME: Record<string, number> = {
	debug: 0,
	info: 1,
	warn: 2,
	warning: 2,
	error: 3,
	err: 3,
	fatal: 4,
};

/** Stored logs carry the serde name (`"Error"`); counts and filters use 0–4. */
export function levelIndex(level: unknown): number {
	if (typeof level === "number") {
		return Number.isInteger(level) && level >= 0 && level < LEVEL_COUNT
			? level
			: 0;
	}
	if (typeof level === "string") return LEVEL_BY_NAME[level.toLowerCase()] ?? 0;
	return 0;
}

export function levelFromName(name: string): number | undefined {
	return LEVEL_BY_NAME[name.trim().toLowerCase()];
}

export function toMicros(time?: ISystemTime | null): number {
	if (!time) return 0;
	return (
		(time.secs_since_epoch ?? 0) * 1_000_000 +
		Math.floor((time.nanos_since_epoch ?? 0) / 1_000)
	);
}

export function logStart(log: ILog): number {
	return toMicros(log.start);
}

export function logDuration(log: ILog): number {
	return Math.max(0, toMicros(log.end) - toMicros(log.start));
}

const pad = (value: number, width = 2) => String(value).padStart(width, "0");

/** Local wall clock, `14:32:09.016`. */
export function formatAbsolute(micros: number): string {
	const date = new Date(Math.floor(micros / 1_000));
	return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`;
}

/** Offset from the run start, `+1.902s`. */
export function formatRelative(micros: number, base: number): string {
	const ms = Math.round((micros - base) / 1_000);
	const sign = ms < 0 ? "-" : "+";
	return `${sign}${(Math.abs(ms) / 1_000).toFixed(3)}s`;
}

export function formatDuration(micros: number): string {
	if (micros <= 0) return "";
	if (micros < 100) return `${micros} µs`;
	const ms = micros / 1_000;
	if (ms < 10) return `${ms.toFixed(1)} ms`;
	if (ms < 1_000) return `${Math.round(ms)} ms`;
	const s = ms / 1_000;
	if (s < 60) return `${s.toFixed(2)} s`;
	const m = Math.floor(s / 60);
	return `${m}m ${Math.round(s - m * 60)}s`;
}

export function firstLine(message: string): { line: string; extra: number } {
	const text = message ?? "";
	const cut = text.indexOf("\n");
	if (cut < 0) return { line: text, extra: 0 };
	let extra = 0;
	for (let i = cut; i < text.length; i++) {
		if (text.charCodeAt(i) === 10) extra++;
	}
	if (text.endsWith("\n")) extra--;
	return { line: text.slice(0, cut).replace(/\r$/, ""), extra };
}

const MAX_JSON_SCAN = 200_000;

function parseJsonContainer(text: string): unknown {
	try {
		const parsed: unknown = JSON.parse(text);
		return parsed !== null && typeof parsed === "object" ? parsed : undefined;
	} catch {
		return undefined;
	}
}

/** The message pretty-printed when all of it is a JSON object or array. */
export function prettyJson(message: string): string | undefined {
	const trimmed = (message ?? "").trim();
	if (trimmed.length > MAX_JSON_SCAN) return undefined;
	const open = trimmed[0];
	const close = trimmed[trimmed.length - 1];
	if (!((open === "{" && close === "}") || (open === "[" && close === "]"))) {
		return undefined;
	}
	const parsed = parseJsonContainer(trimmed);
	return parsed === undefined ? undefined : JSON.stringify(parsed, null, 2);
}

function matchingBracket(text: string, from: number): number {
	const stack: string[] = [];
	let inString = false;
	for (let i = from; i < text.length; i++) {
		const ch = text[i];
		if (inString) {
			if (ch === "\\") i++;
			else if (ch === '"') inString = false;
			continue;
		}
		if (ch === '"') inString = true;
		else if (ch === "{" || ch === "[") stack.push(ch === "{" ? "}" : "]");
		else if (ch === "}" || ch === "]") {
			if (stack.pop() !== ch) return -1;
			if (stack.length === 0) return i;
		}
	}
	return -1;
}

const MAX_EMBED_CANDIDATES = 16;

/** The first non-empty JSON object or array inside a message that is not itself JSON. */
export function embeddedJson(message: string): string | undefined {
	const text = message ?? "";
	if (text.length > MAX_JSON_SCAN || prettyJson(text) !== undefined) {
		return undefined;
	}
	let tried = 0;
	for (let i = 0; i < text.length && tried < MAX_EMBED_CANDIDATES; i++) {
		const ch = text[i];
		if (ch !== "{" && ch !== "[") continue;
		tried++;
		const end = matchingBracket(text, i);
		if (end < 0) continue;
		const parsed = parseJsonContainer(text.slice(i, end + 1));
		const size = Array.isArray(parsed)
			? parsed.length
			: parsed
				? Object.keys(parsed).length
				: 0;
		if (size > 0) return JSON.stringify(parsed, null, 2);
		i = end;
	}
	return undefined;
}

/** Stable across filter changes, so selections survive re-querying. */
export function logKey(log: ILog): string {
	const message = log.message ?? "";
	return [
		logStart(log),
		log.node_id ?? "",
		log.operation_id ?? "",
		levelIndex(log.log_level),
		message.length,
		message.slice(0, 48),
	].join("|");
}

export function sameLog(a: ILog, b: ILog): boolean {
	return (
		logStart(a) === logStart(b) &&
		(a.node_id ?? "") === (b.node_id ?? "") &&
		levelIndex(a.log_level) === levelIndex(b.log_level) &&
		(a.operation_id ?? "") === (b.operation_id ?? "") &&
		a.message === b.message
	);
}
