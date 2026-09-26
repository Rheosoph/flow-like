import type { ILog } from "../../../lib/schema/flow/log";
import {
	LEVEL_LABELS,
	formatAbsolute,
	formatDuration,
	levelIndex,
	logDuration,
	logStart,
	prettyJson,
	toMicros,
} from "./log-format";

export interface ICopyEntry {
	log: ILog;
	node?: string;
	/** The row stands for this many folded occurrences. */
	repeat?: number;
}

export const COPY_LIMIT = 10_000;

function levelLabel(log: ILog): string {
	return LEVEL_LABELS[levelIndex(log.log_level)];
}

function repeatSuffix(repeat?: number): string {
	return repeat && repeat > 1 ? ` (×${repeat})` : "";
}

/** `14:32:09.016 ERROR [Upsert] message`, one entry per line group. */
export function formatText(entries: readonly ICopyEntry[]): string {
	return entries
		.map(({ log, node, repeat }) => {
			const where = node ? ` [${node}]` : "";
			return `${formatAbsolute(logStart(log))} ${levelLabel(log).padEnd(5)}${where} ${log.message ?? ""}${repeatSuffix(repeat)}`;
		})
		.join("\n");
}

export function formatJson(entries: readonly ICopyEntry[]): string {
	const rows = entries.map(({ log, node, repeat }) => {
		const row: Record<string, unknown> = {
			time: formatAbsolute(logStart(log)),
			level: levelLabel(log),
			node: node ?? null,
			node_id: log.node_id ?? null,
			message: log.message ?? "",
			start: toMicros(log.start),
			end: toMicros(log.end),
		};
		if (log.operation_id) row.operation_id = log.operation_id;
		if (log.stats?.token_in != null) row.token_in = log.stats.token_in;
		if (log.stats?.token_out != null) row.token_out = log.stats.token_out;
		if (repeat && repeat > 1) row.repeats = repeat;
		return row;
	});
	return JSON.stringify(rows, null, 2);
}

function longestRun(text: string, ch: string): number {
	let best = 0;
	let run = 0;
	for (const c of text) {
		run = c === ch ? run + 1 : 0;
		if (run > best) best = run;
	}
	return best;
}

function inlineCode(text: string): string {
	const ticks = "`".repeat(longestRun(text, "`") + 1);
	const pad = text.startsWith("`") || text.endsWith("`") ? " " : "";
	return `${ticks}${pad}${text}${pad}${ticks}`;
}

function fenced(text: string, lang: string, indent: string): string {
	const fence = "`".repeat(Math.max(3, longestRun(text, "`") + 1));
	const body = text
		.split("\n")
		.map((line) => `${indent}${line}`)
		.join("\n");
	return `${indent}${fence}${lang}\n${body}\n${indent}${fence}`;
}

/** A bullet per log; multi-line and JSON messages keep their lines in a fence. */
export function formatMarkdown(
	entries: readonly ICopyEntry[],
	title?: string,
): string {
	const lines: string[] = [];
	if (title) lines.push(`### ${title}`, "");
	for (const { log, node, repeat } of entries) {
		const head = [
			`- ${inlineCode(formatAbsolute(logStart(log)))} **${levelLabel(log)}**`,
		];
		if (node) head.push(node);
		const meta: string[] = [];
		if (repeat && repeat > 1) meta.push(`×${repeat}`);
		const duration = formatDuration(logDuration(log));
		if (duration) meta.push(duration);
		lines.push(
			meta.length ? `${head.join(" ")} · ${meta.join(" · ")}` : head.join(" "),
		);
		const message = log.message ?? "";
		const json = prettyJson(message);
		if (json) lines.push(fenced(json, "json", "  "));
		else if (message.includes("\n")) lines.push(fenced(message, "text", "  "));
		else if (message) lines.push(`  ${inlineCode(message)}`);
	}
	return lines.join("\n");
}
