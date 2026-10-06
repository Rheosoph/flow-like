/*
 * Next files (spec M3) and what each host can send inline (spec F): natural name order, files a
 * run on this device already had left out, the Next line, and the size checks at the pick and at
 * the press. Copy is the UI's; these return the numbers it needs.
 */
import {
	FORM_LIMITS,
	type FileSlot,
	type HostCapabilities,
	type LeftOutFile,
} from "../contracts";
import { isFileSlot, isRecord } from "./values";

/** Natural order for file names: 2 before 10, case and accents ignored. */
export function naturalCompare(a: string, b: string): number {
	return a.localeCompare(b, "en", { numeric: true, sensitivity: "base" });
}

export interface SizedFile {
	readonly name: string;
	/** Unknown sizes are null (or 0 from older records). */
	readonly size: number | null;
}

export interface SentFile extends SizedFile {
	/** The run's display number. */
	readonly n: number;
}

/** A run's inputs: a session copy (file slots) or a stored record (`{ $file }` marks). */
export interface SentSource {
	readonly n: number;
	readonly values: Readonly<Record<string, unknown>>;
}

function fileOf(item: unknown): SizedFile | null {
	if (isFileSlot(item)) return { name: item.name, size: item.size };
	if (!isRecord(item) || !isRecord(item.$file)) return null;
	const { name, size } = item.$file;
	if (typeof name !== "string") return null;
	return { name, size: typeof size === "number" ? size : null };
}

function filesIn(value: unknown): readonly SizedFile[] {
	const items = Array.isArray(value) ? value : [value];
	return items.map(fileOf).filter((file): file is SizedFile => file !== null);
}

/**
 * `flpSentFiles`: the files the given runs (newest first) were given in field `name`. The caller
 * passes runs that were sent: not the ones taken out of the queue before their files went.
 */
export function sentFiles(
	runs: readonly SentSource[],
	name: string,
): readonly SentFile[] {
	return runs.flatMap((run) =>
		filesIn(run.values[name]).map((file) => ({ ...file, n: run.n })),
	);
}

/** Same file: same name, and the same size when both sizes are known. */
export function sameFile(a: SizedFile, b: SizedFile): boolean {
	if (a.name !== b.name) return false;
	return !a.size || !b.size || a.size === b.size;
}

export interface QueuedFiles {
	readonly current: FileSlot | null;
	readonly next: readonly FileSlot[];
	/** Picked files (and the current one) a run on this device already had, with that run's number. */
	readonly leftOut: readonly LeftOutFile[];
	/** Files beyond `max` next files ("Only the first 50 files were added."). */
	readonly dropped: number;
}

const byName = (a: FileSlot, b: FileSlot) => naturalCompare(a.name, b.name);

/**
 * `flpQueueFiles`: a pick of several files for a one-file field. Natural name order; files
 * already in the field are skipped; every file a run on this device was already given (the
 * current one too) is left out, so a series never re-sends one; a current file that was not
 * sent stays first; up to `max` next files.
 */
export function queueFiles(
	current: FileSlot | null,
	next: readonly FileSlot[],
	picked: readonly FileSlot[],
	sent: readonly SentFile[],
	max: number = FORM_LIMITS.nextFiles,
): QueuedFiles {
	const sentAs = (file: SizedFile) => sent.find((item) => sameFile(item, file));
	const leftOut: LeftOutFile[] = [];
	const held = current && current.state !== "reminder" ? current : null;
	const heldRun = held ? sentAs(held) : undefined;
	if (held && heldRun) leftOut.push({ slot: held, n: heldRun.n });
	const head = heldRun ? null : held;
	const known: FileSlot[] = [...(head ? [head] : []), ...next];
	const fresh: FileSlot[] = [];
	for (const file of [...picked].sort(byName)) {
		if (known.some((item) => sameFile(item, file))) continue;
		known.push(file);
		const run = sentAs(file);
		if (run) leftOut.push({ slot: file, n: run.n });
		else fresh.push(file);
	}
	const queue = [...next, ...fresh];
	const first = head ?? queue.shift() ?? null;
	return {
		current: first,
		next: queue.slice(0, max),
		leftOut,
		dropped: Math.max(0, queue.length - max),
	};
}

/** `flpNextLine`: "Next: {name} · {more} more", or null when nothing waits. */
export function nextLineParts(
	next: readonly SizedFile[],
): { readonly name: string; readonly more: number } | null {
	return next.length > 0 ? { name: next[0].name, more: next.length - 1 } : null;
}

type InlineHost = Pick<
	HostCapabilities,
	"uploads" | "inlineFileLimitBytes" | "inlineRoomBytes" | "warnFileBytes"
>;

/** The most one file may weigh on an inline host (never above the room of a run); null on upload hosts. */
function fileLimit(host: InlineHost): number | null {
	if (host.uploads !== "inline") return null;
	const room = host.inlineRoomBytes;
	const limit = host.inlineFileLimitBytes ?? room;
	return room !== null && limit !== null ? Math.min(limit, room) : limit;
}

export interface InlineCheck<T extends SizedFile> {
	readonly added: readonly T[];
	/** "{name} is larger than {limit}, the most this link can send. It was not added." */
	readonly refused: readonly {
		readonly name: string;
		readonly limitBytes: number;
	}[];
	/** "{name} is larger than 35 MB. Sending it may fail." */
	readonly warned: readonly { readonly name: string }[];
}

/** `flpInlineCheck`: at the pick, files a host refuses or warns about. */
export function inlineCheck<T extends SizedFile>(
	host: InlineHost,
	files: readonly T[],
): InlineCheck<T> {
	const limit = fileLimit(host);
	const over = (file: T, bytes: number | null) =>
		bytes !== null && file.size !== null && file.size > bytes;
	const added = files.filter((file) => !over(file, limit));
	return {
		added,
		refused: files
			.filter((file) => over(file, limit))
			.map((file) => ({ name: file.name, limitBytes: limit ?? 0 })),
		warned: added
			.filter((file) => over(file, host.warnFileBytes))
			.map((file) => ({ name: file.name })),
	};
}

/**
 * `flpRequestCheck`: at the press, on inline hosts, whether all of a run's files fit in its one
 * request; null when they fit or the host uploads.
 */
export function requestCheck(
	host: Pick<HostCapabilities, "uploads" | "inlineRoomBytes">,
	files: readonly SizedFile[],
): { readonly totalBytes: number; readonly limitBytes: number } | null {
	const room = host.uploads === "inline" ? host.inlineRoomBytes : null;
	if (room === null) return null;
	const totalBytes = files.reduce((sum, file) => sum + (file.size ?? 0), 0);
	return totalBytes > room ? { totalBytes, limitBytes: room } : null;
}
