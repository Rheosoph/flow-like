import type { ILogFold, ILogQuery } from "../../../lib/schema/flow/log-query";
import { ALL_LEVELS, LEVEL_COUNT } from "./log-format";

export type ILogChip =
	| { kind: "node"; value: string; negated: boolean; upstream?: boolean }
	| { kind: "text"; value: string; negated: boolean }
	| { kind: "group"; value: string; label: string; negated: boolean };

export interface ILogFilter {
	/** Levels hidden from the list, sorted. */
	levelsOff: number[];
	chips: ILogChip[];
}

export const EMPTY_FILTER: ILogFilter = { levelsOff: [], chips: [] };

export function chipId(chip: Pick<ILogChip, "kind" | "value">): string {
	return `${chip.kind}:${chip.value}`;
}

function sortedLevels(levels: Iterable<number>): number[] {
	return [...new Set(levels)]
		.filter((level) => level >= 0 && level < LEVEL_COUNT)
		.sort((a, b) => a - b);
}

export function addChip(filter: ILogFilter, chip: ILogChip): ILogFilter {
	const id = chipId(chip);
	const at = filter.chips.findIndex((existing) => chipId(existing) === id);
	if (at < 0) return { ...filter, chips: [...filter.chips, chip] };
	const chips = filter.chips.slice();
	chips[at] = chip;
	return { ...filter, chips };
}

export function removeChip(filter: ILogFilter, id: string): ILogFilter {
	return { ...filter, chips: filter.chips.filter((c) => chipId(c) !== id) };
}

export function toggleChip(filter: ILogFilter, id: string): ILogFilter {
	return {
		...filter,
		chips: filter.chips.map((chip) => {
			if (chipId(chip) !== id) return chip;
			if (chip.kind === "node") {
				return { kind: "node", value: chip.value, negated: !chip.negated };
			}
			return { ...chip, negated: !chip.negated };
		}),
	};
}

export function setChipUpstream(
	filter: ILogFilter,
	id: string,
	upstream: boolean,
): ILogFilter {
	return {
		...filter,
		chips: filter.chips.map((chip) =>
			chipId(chip) === id && chip.kind === "node" && !chip.negated
				? { ...chip, upstream }
				: chip,
		),
	};
}

/** One node in scope: other node includes and an exclusion of it give way. */
export function scopeToNode(filter: ILogFilter, nodeId: string): ILogFilter {
	const chips = filter.chips.filter(
		(chip) => chip.kind !== "node" || (chip.negated && chip.value !== nodeId),
	);
	return {
		...filter,
		chips: [...chips, { kind: "node", value: nodeId, negated: false }],
	};
}

export function excludeNode(filter: ILogFilter, nodeId: string): ILogFilter {
	return addChip(filter, { kind: "node", value: nodeId, negated: true });
}

export function toggleLevel(filter: ILogFilter, level: number): ILogFilter {
	const off = new Set(filter.levelsOff);
	if (off.has(level)) off.delete(level);
	else off.add(level);
	return { ...filter, levelsOff: sortedLevels(off) };
}

/** Alt-click: only this level, or everything again when it already is. */
export function onlyLevel(filter: ILogFilter, level: number): ILogFilter {
	const others = ALL_LEVELS.filter((l) => l !== level);
	const already =
		filter.levelsOff.length === others.length &&
		others.every((l) => filter.levelsOff.includes(l));
	return { ...filter, levelsOff: already ? [] : others };
}

export function invertLevels(filter: ILogFilter): ILogFilter {
	if (filter.levelsOff.length === 0) return filter;
	return {
		...filter,
		levelsOff: ALL_LEVELS.filter((l) => !filter.levelsOff.includes(l)),
	};
}

export function setLevelsOff(filter: ILogFilter, off: number[]): ILogFilter {
	return { ...filter, levelsOff: sortedLevels(off) };
}

export interface ILevelChip {
	negated: boolean;
	levels: number[];
}

/** How the level state reads as a chip: the short list, included or excluded. */
export function levelChip(
	levelsOff: readonly number[],
): ILevelChip | undefined {
	if (levelsOff.length === 0) return undefined;
	const on = ALL_LEVELS.filter((l) => !levelsOff.includes(l));
	if (on.length > 0 && on.length <= 2) return { negated: false, levels: on };
	return { negated: true, levels: [...levelsOff] };
}

export function hasTextInclude(chips: readonly ILogChip[]): boolean {
	return chips.some((chip) => chip.kind === "text" && !chip.negated);
}

export function isFilterEmpty(filter: ILogFilter): boolean {
	return filter.levelsOff.length === 0 && filter.chips.length === 0;
}

export function includedGroups(chips: readonly ILogChip[]): string[] {
	return chips
		.filter((chip) => chip.kind === "group" && !chip.negated)
		.map((chip) => chip.value);
}

function pushUnique(list: string[] | undefined, value: string): string[] {
	const out = list ?? [];
	if (!out.includes(value)) out.push(value);
	return out;
}

export interface IFilterToQueryOptions {
	upstream?: (nodeId: string) => readonly string[];
	fold?: readonly ILogFold[];
}

export function filterToQuery(
	filter: ILogFilter,
	options: IFilterToQueryOptions = {},
): ILogQuery {
	const query: ILogQuery = {};
	if (filter.levelsOff.length > 0) {
		query.exclude_levels = sortedLevels(filter.levelsOff);
	}
	for (const chip of filter.chips) {
		const value = chip.value;
		if (!value) continue;
		switch (chip.kind) {
			case "node":
				if (chip.negated) {
					query.exclude_nodes = pushUnique(query.exclude_nodes, value);
					break;
				}
				query.nodes = pushUnique(query.nodes, value);
				if (chip.upstream && options.upstream) {
					for (const id of options.upstream(value)) {
						query.nodes = pushUnique(query.nodes, id);
					}
				}
				break;
			case "text":
				if (chip.negated) {
					query.exclude_text = pushUnique(query.exclude_text, value);
				} else query.text = pushUnique(query.text, value);
				break;
			case "group":
				if (chip.negated) {
					query.exclude_fingerprints = pushUnique(
						query.exclude_fingerprints,
						value,
					);
				} else query.fingerprints = pushUnique(query.fingerprints, value);
				break;
		}
	}
	if (options.fold && options.fold.length > 0) {
		query.fold = options.fold.map((f) => ({ ...f }));
	}
	return query;
}

/** Stable identity for caches and query keys: arrays keep their order. */
export function queryKey(query: ILogQuery): string {
	return JSON.stringify(
		Object.fromEntries(
			Object.entries(query)
				.filter(([, value]) => value !== undefined)
				.sort(([a], [b]) => a.localeCompare(b)),
		),
	);
}

export function withTimeRange(
	query: ILogQuery,
	from?: number,
	to?: number,
): ILogQuery {
	const out: ILogQuery = { ...query };
	if (from !== undefined) out.from = from;
	if (to !== undefined) out.to = to;
	return out;
}

/** Narrows a query to error and fatal rows, keeping every other filter. */
export function errorsOnly(query: ILogQuery): ILogQuery {
	const excluded = new Set([...(query.exclude_levels ?? []), 0, 1, 2]);
	return { ...query, exclude_levels: sortedLevels(excluded) };
}
