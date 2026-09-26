import type { ILog } from "../../../lib/schema/flow/log";
import type {
	ILogFold,
	ILogGroup,
	ILogSlotRange,
	IRunLogSummary,
} from "../../../lib/schema/flow/log-query";
import { logStart } from "./log-format";

export const FOLD_LIMIT = 64;

export interface IFoldOptions {
	enabled: boolean;
	unfolded: ReadonlySet<string>;
	/** A text include filter is active: a group's first row may not match it. */
	textActive: boolean;
	/** Groups the user scoped to; folding them would leave one row. */
	onlyGroups?: readonly string[];
}

export interface IFoldPlan {
	fold: ILogFold[];
	paused: boolean;
}

export function planFold(
	summary: IRunLogSummary | null | undefined,
	options: IFoldOptions,
): IFoldPlan {
	if (!summary?.fingerprinted || !options.enabled) {
		return { fold: [], paused: false };
	}
	if (options.textActive) return { fold: [], paused: true };
	const skip = new Set(options.onlyGroups ?? []);
	const fold = summary.groups
		.filter(
			(group) =>
				group.count >= 2 &&
				!options.unfolded.has(group.fingerprint) &&
				!skip.has(group.fingerprint),
		)
		.sort((a, b) => b.count - a.count || a.first_start - b.first_start)
		.slice(0, FOLD_LIMIT)
		.map((group) => ({
			fingerprint: group.fingerprint,
			first_start: group.first_start,
		}));
	return { fold, paused: false };
}

export function groupsByFingerprint(
	summary: IRunLogSummary | null | undefined,
): Map<string, ILogGroup> {
	const map = new Map<string, ILogGroup>();
	for (const group of summary?.groups ?? []) map.set(group.fingerprint, group);
	return map;
}

/** The group a row stands in for, when it is the kept first occurrence. */
export function foldHead(
	log: ILog,
	folded: ReadonlySet<string>,
	groups: ReadonlyMap<string, ILogGroup>,
): ILogGroup | undefined {
	const fingerprint = log.fingerprint;
	if (!fingerprint || !folded.has(fingerprint)) return undefined;
	const group = groups.get(fingerprint);
	if (!group || group.first_start !== logStart(log)) return undefined;
	return group;
}

export type ITemplatePart =
	| { kind: "text"; text: string }
	| { kind: "slot"; label: string };

export function slotLabel(range: ILogSlotRange | null | undefined): string {
	if (!range) return "⟨n⟩";
	if (range.min === range.max) return `⟨${range.min}⟩`;
	return `⟨${range.min}…${range.max}⟩`;
}

const PLACEHOLDER = /⟨(n|id)⟩/g;

/** Splits a group template at its placeholders, labelling `⟨n⟩` with the range seen. */
export function templateParts(
	template: string,
	slots: ReadonlyArray<ILogSlotRange | null>,
): ITemplatePart[] {
	const parts: ITemplatePart[] = [];
	let cursor = 0;
	let numberSlot = 0;
	for (const match of template.matchAll(PLACEHOLDER)) {
		const at = match.index ?? 0;
		if (at > cursor)
			parts.push({ kind: "text", text: template.slice(cursor, at) });
		if (match[1] === "n") {
			parts.push({ kind: "slot", label: slotLabel(slots[numberSlot]) });
			numberSlot++;
		} else {
			parts.push({ kind: "slot", label: "⟨id⟩" });
		}
		cursor = at + match[0].length;
	}
	if (cursor < template.length) {
		parts.push({ kind: "text", text: template.slice(cursor) });
	}
	return parts;
}

/** A short, readable name for a group chip. */
export function groupLabel(template: string, max = 36): string {
	const line = template.split("\n", 1)[0] ?? "";
	return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}
