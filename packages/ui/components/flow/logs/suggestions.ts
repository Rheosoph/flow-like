import type { ILogGroup } from "../../../lib/schema/flow/log-query";
import { type ILogFilter, addChip, setLevelsOff } from "./filter-model";
import { ALL_LEVELS, LEVEL_NAMES } from "./log-format";
import {
	type INodeRef,
	type IParsedInput,
	levelsAfter,
	parseQueryInput,
	trailingToken,
} from "./query-tokens";

export type ISuggestion =
	| { id: string; kind: "text"; text: string; negated: boolean }
	| {
			id: string;
			kind: "node";
			nodeId: string;
			name: string;
			count: number;
			negated: boolean;
	  }
	| {
			id: string;
			kind: "level";
			level: number;
			count: number;
			negated: boolean;
	  }
	| {
			id: string;
			kind: "group";
			fingerprint: string;
			template: string;
			count: number;
			negated: boolean;
	  };

export interface ISuggestionContext {
	nodes: readonly INodeRef[];
	/** Node id → logs in the run. */
	nodeCounts: Readonly<Record<string, number>>;
	levelCounts: readonly number[];
	groups: readonly ILogGroup[];
}

const NODE_LIMIT = 8;
const GROUP_LIMIT = 5;

const sign = (negated: boolean) => (negated ? "-" : "");

function nodeSuggestions(
	query: string,
	negated: boolean,
	ctx: ISuggestionContext,
	limit: number,
): ISuggestion[] {
	const wanted = query.toLowerCase();
	return ctx.nodes
		.map((node) => {
			const name = node.friendlyName || node.name;
			const lower = name.toLowerCase();
			const rank = !wanted
				? 1
				: lower.startsWith(wanted)
					? 0
					: lower.includes(wanted) || node.name.toLowerCase().includes(wanted)
						? 1
						: -1;
			return { node, name, rank, count: ctx.nodeCounts[node.id] ?? 0 };
		})
		.filter((entry) => entry.rank >= 0 && (wanted || entry.count > 0))
		.sort(
			(a, b) =>
				a.rank - b.rank || b.count - a.count || a.name.localeCompare(b.name),
		)
		.slice(0, limit)
		.map(({ node, name, count }) => ({
			id: `node:${sign(negated)}${node.id}`,
			kind: "node" as const,
			nodeId: node.id,
			name,
			count,
			negated,
		}));
}

function levelSuggestions(
	query: string,
	negated: boolean,
	ctx: ISuggestionContext,
): ISuggestion[] {
	const wanted = query.toLowerCase();
	return ALL_LEVELS.filter((level) =>
		wanted
			? LEVEL_NAMES[level].startsWith(wanted)
			: (ctx.levelCounts[level] ?? 0) > 0,
	).map((level) => ({
		id: `level:${sign(negated)}${level}`,
		kind: "level" as const,
		level,
		count: ctx.levelCounts[level] ?? 0,
		negated,
	}));
}

function groupSuggestions(
	query: string,
	negated: boolean,
	ctx: ISuggestionContext,
): ISuggestion[] {
	const wanted = query.toLowerCase();
	if (!wanted) return [];
	return ctx.groups
		.filter(
			(group) =>
				group.count >= 2 && group.template.toLowerCase().includes(wanted),
		)
		.slice(0, GROUP_LIMIT)
		.map((group) => ({
			id: `group:${sign(negated)}${group.fingerprint}`,
			kind: "group" as const,
			fingerprint: group.fingerprint,
			template: group.template,
			count: group.count,
			negated,
		}));
}

/** What the token being typed could become, the likeliest first. */
export function buildSuggestions(
	input: string,
	ctx: ISuggestionContext,
): ISuggestion[] {
	const { token } = trailingToken(input);
	if (!token) {
		return [
			...nodeSuggestions("", false, ctx, 6),
			...levelSuggestions("", false, ctx),
		];
	}
	const { negated, value } = token;
	if (token.key === "node")
		return nodeSuggestions(value, negated, ctx, NODE_LIMIT);
	if (token.key === "level") return levelSuggestions(value, negated, ctx);
	if (!value || (value === "-" && !token.quoted)) {
		return nodeSuggestions("", negated || value === "-", ctx, 6);
	}
	const text = (polarity: boolean): ISuggestion => ({
		id: `text:${sign(polarity)}${value}`,
		kind: "text",
		text: value,
		negated: polarity,
	});
	return [
		text(negated),
		text(!negated),
		...groupSuggestions(value, negated, ctx),
		...nodeSuggestions(value, negated, ctx, 5),
		...levelSuggestions(value, negated, ctx),
	];
}

export function applyParsed(
	filter: ILogFilter,
	parsed: IParsedInput,
): ILogFilter {
	let next = filter;
	if (parsed.onlyLevels.length > 0 || parsed.hideLevels.length > 0) {
		next = setLevelsOff(next, levelsAfter(next.levelsOff, parsed));
	}
	for (const chip of parsed.chips) next = addChip(next, chip);
	return next;
}

/** Commits whatever precedes the trailing token, then the chosen suggestion. */
export function applySuggestion(
	filter: ILogFilter,
	input: string,
	suggestion: ISuggestion,
	nodes: readonly INodeRef[],
	groupLabel: (template: string) => string,
): ILogFilter {
	const { prefix } = trailingToken(input);
	const next = applyParsed(
		filter,
		parseQueryInput(prefix, nodes, { commit: true }),
	);
	switch (suggestion.kind) {
		case "text":
			return addChip(next, {
				kind: "text",
				value: suggestion.text,
				negated: suggestion.negated,
			});
		case "node":
			return addChip(next, {
				kind: "node",
				value: suggestion.nodeId,
				negated: suggestion.negated,
			});
		case "group":
			return addChip(next, {
				kind: "group",
				value: suggestion.fingerprint,
				label: groupLabel(suggestion.template),
				negated: suggestion.negated,
			});
		case "level":
			return setLevelsOff(
				next,
				levelsAfter(next.levelsOff, {
					onlyLevels: suggestion.negated ? [] : [suggestion.level],
					hideLevels: suggestion.negated ? [suggestion.level] : [],
				}),
			);
	}
}
