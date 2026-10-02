import type {
	RecordStream,
	TelemetryRecord,
} from "../../../../lib/device-management/workspace/streams";
import type { StreamGap } from "../../../../lib/device-management/workspace/types";
import { humanFileSize } from "../../../../lib/utils";
import type { LogRecord } from "../primitives/log-viewer";

/**
 * The app's base style gives every `<p>` a 28 px leading; under this class a
 * `<p>` without a leading of its own follows its container (zero specificity,
 * so any text utility still wins).
 */
export const PLAIN_PARAGRAPHS = "[:where(&_p)]:leading-[inherit]";
/** Blocks of a tab, one under the other. */
export const OBSERVE_STACK = `flex min-w-0 flex-col gap-4 ${PLAIN_PARAGRAPHS}`;
/** Two blocks side by side from 980 px of area width (SPEC §5.2 `.cols-2`). */
export const OBSERVE_COLS =
	"grid min-w-0 items-start gap-x-6 gap-y-4 @min-[980px]/devices:grid-cols-2";

/** Undoes the app-wide `table`, `th` and `td` rules (margins, cell borders) inside a `DvTable`. */
export const TABLE_RESET =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0";

export type Json = Record<string, unknown>;

export const asRecord = (value: unknown): Json | undefined =>
	value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Json)
		: undefined;

/** A finite, non-negative number; anything else is "the device didn't say". */
export const amount = (value: unknown): number | undefined =>
	typeof value === "number" && Number.isFinite(value) && value >= 0
		? value
		: undefined;

export const whole = (value: unknown): number | undefined =>
	Number.isSafeInteger(value) && Number(value) >= 0 ? Number(value) : undefined;

export const word = (value: unknown): string | undefined =>
	typeof value === "string" && value !== "" ? value : undefined;

export const GIB = 1024 ** 3;
export const MIB = 1024 ** 2;

export interface MetricSample {
	/** Unix seconds. */
	at: number;
	data: Json;
}

/** Samples of a metrics read, oldest first. A read without records is one sample taken at `fallbackAt`. */
export function metricSamples(
	source: unknown,
	fallbackAt?: number,
): MetricSample[] {
	const read = asRecord(source);
	if (!read) return [];
	const rows = Array.isArray(read.records) ? read.records : [];
	const samples = rows.flatMap((row) => {
		const entry = asRecord(row);
		const data = asRecord(entry?.data);
		const at = amount(entry?.timestamp);
		return data && at !== undefined ? [{ at, data }] : [];
	});
	if (samples.length) return samples.sort((a, b) => a.at - b.at);
	return fallbackAt !== undefined && Object.keys(read).length
		? [{ at: fallbackAt, data: read }]
		: [];
}

/** The trend of the encrypted snapshot, continued by the live samples that are newer. */
export function mergeSamples(
	trend: readonly MetricSample[],
	live: readonly MetricSample[],
): MetricSample[] {
	const newest = trend.at(-1)?.at ?? 0;
	return [...trend, ...live.filter((sample) => sample.at > newest)];
}

export function seriesOf(
	samples: readonly MetricSample[],
	read: (data: Json) => number | undefined,
): number[] {
	return samples.map((sample) => read(sample.data) ?? Number.NaN);
}

/** Seconds between two points of a trend; undefined for a single sample. */
export function stepOf(samples: readonly MetricSample[]): number | undefined {
	const first = samples[0];
	const last = samples.at(-1);
	if (!first || !last || samples.length < 2) return undefined;
	return Math.max(1, Math.round((last.at - first.at) / (samples.length - 1)));
}

/** Records of a log or activity stream, oldest first. */
export function recordsOf(data: unknown): TelemetryRecord[] {
	const rows = (data as RecordStream | undefined)?.records;
	if (!Array.isArray(rows)) return [];
	return rows.filter(
		(row): row is TelemetryRecord =>
			!!row &&
			Number.isSafeInteger(row.sequence) &&
			Number.isFinite(row.timestamp) &&
			!!asRecord(row.data),
	);
}

export const metaOf = (data: unknown): Json =>
	asRecord((data as RecordStream | undefined)?.meta) ?? {};

/** A record that only reports dropped lines is a marker, any other a line. */
function logRow(row: TelemetryRecord): LogRecord {
	const dropped = whole(row.data.dropped_lines);
	if (dropped)
		return {
			kind: "gap",
			id: `d${row.sequence}`,
			reason: "dropped",
			count: dropped,
		};
	return {
		kind: "line",
		id: String(row.sequence),
		at: row.timestamp,
		stream: row.data.stream === "stderr" ? "stderr" : "stdout",
		message: word(row.data.message) ?? "",
		truncated: row.data.truncated === true,
	};
}

/** Lines and markers of a log read, oldest first. */
export function logRecords(
	records: readonly TelemetryRecord[],
	/** The newest record the device deleted; pass it once nothing older can be loaded. */
	evictedThrough?: number,
): LogRecord[] {
	const deleted: LogRecord[] =
		evictedThrough === undefined
			? []
			: [
					{
						kind: "gap",
						id: "evicted",
						reason: "evicted",
						before: evictedThrough + 1,
					},
				];
	return [...deleted, ...records.map(logRow)];
}

/** The newest marker of one kind; streams keep at most 64. */
export function lastGap(
	gaps: readonly StreamGap[],
	kind: StreamGap["kind"],
): StreamGap | undefined {
	return gaps.findLast((gap) => gap.kind === kind);
}

export function sumGaps(
	gaps: readonly StreamGap[],
	kind: StreamGap["kind"],
): number {
	return gaps
		.filter((gap) => gap.kind === kind)
		.reduce((total, gap) => total + (gap.count ?? 0), 0);
}

/** "1.2" and "GiB" apart, so the unit can sit in the muted unit slot of a metric. */
export function bytesParts(value: number): [amount: string, unit: string] {
	const [size = "", unit = ""] = humanFileSize(Math.round(value)).split(" ");
	return [size, unit];
}

export const bytesText = (value: number): string =>
	humanFileSize(Math.round(value));

/** "1 Sept" (with the year when it isn't this one): a calendar day in the viewer's zone. */
export function dayLabel(
	time: { now: number; locale: string },
	atS: number,
): string {
	const date = new Date(atS * 1000);
	const sameYear = date.getFullYear() === new Date(time.now).getFullYear();
	return new Intl.DateTimeFormat(time.locale, {
		day: "numeric",
		month: "short",
		...(sameYear ? {} : { year: "numeric" }),
	}).format(date);
}

/** Seconds a list of samples spans. */
export const spanOf = (samples: readonly MetricSample[]): number =>
	samples.length > 1 ? (samples.at(-1)?.at ?? 0) - (samples[0]?.at ?? 0) : 0;

/** File-name safe form of a device or service name. */
export const fileSlug = (name: string): string =>
	name
		.toLowerCase()
		.replace(/[^a-z0-9._-]+/g, "-")
		.replace(/^-+|-+$/g, "") || "device";

/** Offers `text` as a file; false when this window can't create downloads. */
export function downloadText(
	fileName: string,
	text: string,
	type = "text/plain",
): boolean {
	const view = globalThis.document;
	if (!view || typeof URL.createObjectURL !== "function") return false;
	const url = URL.createObjectURL(new Blob([text], { type }));
	const link = view.createElement("a");
	link.href = url;
	link.download = fileName;
	link.rel = "noopener";
	view.body.append(link);
	link.click();
	link.remove();
	setTimeout(() => URL.revokeObjectURL(url), 0);
	return true;
}
