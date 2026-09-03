import type { IEventTimelineEntry } from "../state/backend-state/event-state";
import type { ILogMetadata } from "./schema/flow/log-metadata";

/**
 * Aggregation helpers for the event History section.
 *
 * Two version formats coexist in the run store and must never be mixed up:
 * `event_version` is dotted `"major.minor.patch"` (the timeline's
 * `version_key`), while the board `version` is `"v{major}-{minor}-{patch}"`.
 * Each parser accepts exactly one of them.
 */

const DOTTED_EVENT_VERSION = /^(\d+)\.(\d+)\.(\d+)$/;
const BOARD_VERSION = /^v(\d+)-(\d+)-(\d+)$/;

/** Parses a dotted event version key (`"1.0.4"`). Rejects the board format. */
export function parseDottedEventVersion(
	value: string | null | undefined,
): [number, number, number] | null {
	const match = value?.trim().match(DOTTED_EVENT_VERSION);
	if (!match) return null;
	return [Number(match[1]), Number(match[2]), Number(match[3])];
}

/** Parses a board version string (`"v1-0-4"`). Rejects the dotted format. */
export function parseBoardVersionString(
	value: string | null | undefined,
): [number, number, number] | null {
	const match = value?.trim().match(BOARD_VERSION);
	if (!match) return null;
	return [Number(match[1]), Number(match[2]), Number(match[3])];
}

/** The dotted key runs are grouped by — mirrors the API's `version_key`. */
export function toDottedVersionKey(version: readonly number[]): string {
	return version.join(".");
}

export interface IEventVersionRunAggregate {
	/** Dotted event version, or null for runs recorded without one. */
	versionKey: string | null;
	total: number;
	ok: number;
	warn: number;
	fail: number;
	/** Durations in microseconds, from `end - start`. */
	p50DurationUs: number;
	p95DurationUs: number;
	/** Earliest run start (microseconds since epoch). */
	firstSeen: number;
	/** Latest run start (microseconds since epoch). */
	lastSeen: number;
	/** Distinct board `version` strings as stored (`"v1-0-4"`), first-seen order. */
	boardVersions: string[];
}

function severityBucket(logLevel: number): "ok" | "warn" | "fail" {
	if (logLevel >= 3) return "fail";
	if (logLevel === 2) return "warn";
	return "ok";
}

/** Nearest-rank percentile over an ascending-sorted array. */
function percentile(sorted: number[], p: number): number {
	if (sorted.length === 0) return 0;
	const rank = Math.ceil((p / 100) * sorted.length);
	return sorted[Math.min(sorted.length - 1, Math.max(0, rank - 1))];
}

function compareVersionKeysDesc(a: string | null, b: string | null): number {
	if (a === b) return 0;
	// Unversioned runs sort last — they predate version tracking.
	if (a === null) return 1;
	if (b === null) return -1;
	const parsedA = parseDottedEventVersion(a);
	const parsedB = parseDottedEventVersion(b);
	if (parsedA && parsedB) {
		for (let i = 0; i < 3; i++) {
			if (parsedA[i] !== parsedB[i]) return parsedB[i] - parsedA[i];
		}
		return 0;
	}
	if (parsedA) return -1;
	if (parsedB) return 1;
	return b.localeCompare(a);
}

/**
 * Groups run summaries by their dotted `event_version` key. Runs without one
 * (recorded before version stamping landed) collect under `versionKey: null`.
 */
export function aggregateRunsByEventVersion(
	runs: readonly ILogMetadata[],
): IEventVersionRunAggregate[] {
	const groups = new Map<
		string | null,
		{ durations: number[]; aggregate: IEventVersionRunAggregate }
	>();

	for (const run of runs) {
		const versionKey = parseDottedEventVersion(run.event_version)
			? (run.event_version as string)
			: null;
		let group = groups.get(versionKey);
		if (!group) {
			group = {
				durations: [],
				aggregate: {
					versionKey,
					total: 0,
					ok: 0,
					warn: 0,
					fail: 0,
					p50DurationUs: 0,
					p95DurationUs: 0,
					firstSeen: run.start,
					lastSeen: run.start,
					boardVersions: [],
				},
			};
			groups.set(versionKey, group);
		}

		const aggregate = group.aggregate;
		aggregate.total += 1;
		aggregate[severityBucket(run.log_level)] += 1;
		group.durations.push(Math.abs(run.end - run.start));
		aggregate.firstSeen = Math.min(aggregate.firstSeen, run.start);
		aggregate.lastSeen = Math.max(aggregate.lastSeen, run.start);
		if (run.version && !aggregate.boardVersions.includes(run.version)) {
			aggregate.boardVersions.push(run.version);
		}
	}

	const aggregates: IEventVersionRunAggregate[] = [];
	for (const group of groups.values()) {
		group.durations.sort((a, b) => a - b);
		group.aggregate.p50DurationUs = percentile(group.durations, 50);
		group.aggregate.p95DurationUs = percentile(group.durations, 95);
		aggregates.push(group.aggregate);
	}
	aggregates.sort((a, b) => compareVersionKeysDesc(a.versionKey, b.versionKey));
	return aggregates;
}

export interface INodeSeverityAggregate {
	nodeId: string;
	/** Runs this node appeared in — not per-node call counts. */
	visits: number;
	/** Highest numeric log level seen for the node across runs. */
	worstLevel: number;
	warnRuns: number;
	failRuns: number;
}

/**
 * Per-node visit counts and worst severity from the summaries' `nodes` lists
 * (written at every log level; per-node durations exist only at Debug).
 */
export function aggregateNodeSeverity(
	runs: readonly ILogMetadata[],
): INodeSeverityAggregate[] {
	const byNode = new Map<string, INodeSeverityAggregate>();

	for (const run of runs) {
		for (const entry of run.nodes ?? []) {
			const [nodeId, level] = entry;
			if (typeof nodeId !== "string" || typeof level !== "number") continue;
			let aggregate = byNode.get(nodeId);
			if (!aggregate) {
				aggregate = {
					nodeId,
					visits: 0,
					worstLevel: level,
					warnRuns: 0,
					failRuns: 0,
				};
				byNode.set(nodeId, aggregate);
			}
			aggregate.visits += 1;
			aggregate.worstLevel = Math.max(aggregate.worstLevel, level);
			const bucket = severityBucket(level);
			if (bucket === "warn") aggregate.warnRuns += 1;
			if (bucket === "fail") aggregate.failRuns += 1;
		}
	}

	return Array.from(byNode.values()).sort(
		(a, b) =>
			b.worstLevel - a.worstLevel ||
			b.failRuns - a.failRuns ||
			b.visits - a.visits ||
			a.nodeId.localeCompare(b.nodeId),
	);
}

export interface ITimelineEntryDiff {
	/** Stable field id — the component maps it to a translated label. */
	field: string;
	from: string;
	to: string;
}

const NONE = "—";

const DIFF_FIELDS: Array<[string, (entry: IEventTimelineEntry) => string]> = [
	["name", (entry) => entry.name || NONE],
	["description", (entry) => entry.description || NONE],
	["event_type", (entry) => entry.event_type],
	["active", (entry) => String(entry.active)],
	["board", (entry) => entry.board_id ?? NONE],
	[
		"board_version",
		(entry) =>
			entry.board_version ? `v${entry.board_version.join(".")}` : "latest",
	],
	["node", (entry) => entry.node_id ?? NONE],
	["page", (entry) => entry.default_page_id ?? NONE],
	["route", (entry) => entry.route ?? NONE],
	["is_default", (entry) => String(entry.is_default)],
	["execution_mode", (entry) => entry.execution_mode],
	["exposure", (entry) => entry.exposure],
	["variables", (entry) => entry.variable_ids.join(", ") || NONE],
	["secret_variables", (entry) => entry.secret_variable_ids.join(", ") || NONE],
	["notes", (entry) => entry.notes_kind ?? NONE],
];

/**
 * Field-level differences between two timeline entries. The timeline carries
 * no config bytes, so type-specific config changes are invisible here.
 */
export function diffTimelineEntries(
	from: IEventTimelineEntry,
	to: IEventTimelineEntry,
): ITimelineEntryDiff[] {
	const diffs: ITimelineEntryDiff[] = [];
	for (const [field, project] of DIFF_FIELDS) {
		const fromValue = project(from);
		const toValue = project(to);
		if (fromValue !== toValue) {
			diffs.push({ field, from: fromValue, to: toValue });
		}
	}
	return diffs;
}
