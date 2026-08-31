"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Filter, FilterX, X } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { SubgraphNode } from "../../../state/backend-state/graph-state";
import { Button } from "../button";
import { ScrollArea } from "../scroll-area";

/** Facets offered at once; more reads as a form, not a summary. */
const MAX_FACETS = 10;
/** Values listed per facet before the tail folds into one row. */
const MAX_VALUES_PER_FACET = 8;
/** Distinct string values above which a property is an identifier, not a facet. */
const MAX_DISTINCT_STRING_VALUES = 24;
/** Share of nodes that must carry a property before it is worth faceting. */
const MIN_COVERAGE = 0.25;
/** Buckets used for numeric ranges. */
const NUMERIC_BINS = 6;
/** Value text longer than this is free text, not a category. */
const MAX_VALUE_LENGTH = 80;

function selectionKey(facetKey: string, valueKey: string): string {
	return `${facetKey}::${valueKey}`;
}

interface FacetValue {
	key: string;
	label: string;
	count: number;
	ids: string[];
}

interface Facet {
	key: string;
	title: string;
	values: FacetValue[];
}

export interface GraphHistogramPanelProps {
	nodes: readonly SubgraphNode[];
	onClose: () => void;
	/** Live highlight while a row is hovered; null clears it. */
	onHoverValue: (ids: Set<string> | null) => void;
	onFilterTo: (title: string, ids: Set<string>) => void;
	onFilterOut: (title: string, ids: Set<string>) => void;
}

function formatBinLabel(low: number, high: number): string {
	const fmt = (value: number) =>
		Math.abs(value) >= 1000
			? value.toLocaleString(undefined, { maximumFractionDigits: 0 })
			: value.toLocaleString(undefined, { maximumFractionDigits: 2 });
	return `${fmt(low)} – ${fmt(high)}`;
}

function buildFacets(nodes: readonly SubgraphNode[]): Facet[] {
	if (nodes.length === 0) return [];

	const facets: Facet[] = [];

	const byLabel = new Map<string, string[]>();
	for (const node of nodes) {
		const bucket = byLabel.get(node.label);
		if (bucket) bucket.push(node.id);
		else byLabel.set(node.label, [node.id]);
	}
	facets.push({
		key: "__label__",
		title: "Object type",
		values: [...byLabel.entries()]
			.sort((a, b) => b[1].length - a[1].length)
			.map(([label, ids]) => ({
				key: label,
				label,
				count: ids.length,
				ids,
			})),
	});

	// Property facets, scored by how much of the sample they describe.
	const propertyValues = new Map<string, Map<string, { ids: string[] }>>();
	const numericValues = new Map<string, { id: string; value: number }[]>();
	const coverage = new Map<string, number>();

	for (const node of nodes) {
		for (const [key, raw] of Object.entries(node.props ?? {})) {
			if (raw === null || raw === undefined) continue;
			coverage.set(key, (coverage.get(key) ?? 0) + 1);

			if (typeof raw === "number" && Number.isFinite(raw)) {
				const bucket = numericValues.get(key);
				if (bucket) bucket.push({ id: node.id, value: raw });
				else numericValues.set(key, [{ id: node.id, value: raw }]);
				continue;
			}
			if (typeof raw === "boolean" || typeof raw === "string") {
				const text = String(raw);
				if (text.length === 0 || text.length > MAX_VALUE_LENGTH) continue;
				let values = propertyValues.get(key);
				if (!values) {
					values = new Map();
					propertyValues.set(key, values);
				}
				const entry = values.get(text);
				if (entry) entry.ids.push(node.id);
				else values.set(text, { ids: [node.id] });
			}
		}
	}

	const minCovered = Math.max(2, Math.ceil(nodes.length * MIN_COVERAGE));
	const candidates: { key: string; covered: number; facet: Facet }[] = [];

	for (const [key, values] of propertyValues) {
		const covered = coverage.get(key) ?? 0;
		if (covered < minCovered) continue;
		if (values.size < 2 || values.size > MAX_DISTINCT_STRING_VALUES) continue;

		const sorted = [...values.entries()].sort(
			(a, b) => b[1].ids.length - a[1].ids.length,
		);
		const head = sorted.slice(0, MAX_VALUES_PER_FACET);
		const tail = sorted.slice(MAX_VALUES_PER_FACET);
		const facetValues: FacetValue[] = head.map(([text, entry]) => ({
			key: text,
			label: text,
			count: entry.ids.length,
			ids: entry.ids,
		}));
		if (tail.length > 0) {
			const ids = tail.flatMap(([, entry]) => entry.ids);
			facetValues.push({
				key: "__other__",
				label: `Other (${tail.length.toLocaleString()})`,
				count: ids.length,
				ids,
			});
		}
		candidates.push({
			key,
			covered,
			facet: { key, title: key, values: facetValues },
		});
	}

	for (const [key, entries] of numericValues) {
		const covered = coverage.get(key) ?? 0;
		if (covered < minCovered || entries.length < 2) continue;
		let low = Number.POSITIVE_INFINITY;
		let high = Number.NEGATIVE_INFINITY;
		for (const entry of entries) {
			low = Math.min(low, entry.value);
			high = Math.max(high, entry.value);
		}
		if (!(high > low)) continue;

		const step = (high - low) / NUMERIC_BINS;
		const bins: FacetValue[] = Array.from({ length: NUMERIC_BINS }, (_, i) => ({
			key: `bin-${i}`,
			label: formatBinLabel(low + i * step, low + (i + 1) * step),
			count: 0,
			ids: [],
		}));
		for (const entry of entries) {
			const index = Math.min(
				NUMERIC_BINS - 1,
				Math.floor((entry.value - low) / step),
			);
			bins[index].count += 1;
			bins[index].ids.push(entry.id);
		}
		candidates.push({
			key,
			covered,
			facet: {
				key,
				title: key,
				values: bins.filter((bin) => bin.count > 0),
			},
		});
	}

	candidates.sort((a, b) => b.covered - a.covered || (a.key < b.key ? -1 : 1));
	for (const candidate of candidates.slice(0, MAX_FACETS - 1)) {
		facets.push(candidate.facet);
	}

	return facets;
}

/**
 * Value distributions over the loaded sample, wired to the canvas: hover a bar
 * to see who it is, select bars and filter the stage to (or without) them.
 * Reading the data's shape here beats squinting at four hundred circles.
 */
export function GraphHistogramPanel({
	nodes,
	onClose,
	onHoverValue,
	onFilterTo,
	onFilterOut,
}: GraphHistogramPanelProps) {
	const { t } = useTranslation("common");
	const [selected, setSelected] = useState<Set<string>>(new Set());

	const facets = useMemo(() => buildFacets(nodes), [nodes]);

	const selectedIds = useMemo(() => {
		if (selected.size === 0) return null;
		const ids = new Set<string>();
		let title = "";
		for (const facet of facets) {
			for (const value of facet.values) {
				if (!selected.has(selectionKey(facet.key, value.key))) continue;
				for (const id of value.ids) ids.add(id);
				title = title
					? `${title} + ${value.label}`
					: `${facet.title}: ${value.label}`;
			}
		}
		return { ids, title };
	}, [selected, facets]);

	const toggle = useCallback((facetKey: string, valueKey: string) => {
		setSelected((prev) => {
			const next = new Set(prev);
			const key = selectionKey(facetKey, valueKey);
			if (next.has(key)) next.delete(key);
			else next.add(key);
			return next;
		});
	}, []);

	const applyFilter = useCallback(
		(mode: "to" | "out") => {
			if (!selectedIds || selectedIds.ids.size === 0) return;
			if (mode === "to") onFilterTo(selectedIds.title, selectedIds.ids);
			else onFilterOut(selectedIds.title, selectedIds.ids);
			setSelected(new Set());
			onHoverValue(null);
		},
		[selectedIds, onFilterTo, onFilterOut, onHoverValue],
	);

	return (
		<div className="flex h-full w-72 shrink-0 flex-col overflow-hidden border-l bg-background animate-in slide-in-from-right-5 duration-200">
			<div className="flex shrink-0 items-center justify-between border-b p-3">
				<p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
					{t("facets", "Facets")}
				</p>
				<Button
					variant="ghost"
					size="icon"
					className="h-7 w-7"
					onClick={onClose}
				>
					<X className="h-4 w-4" />
				</Button>
			</div>

			<ScrollArea className="min-h-0 flex-1">
				<div className="space-y-4 p-3">
					{facets.length === 0 && (
						<p className="text-xs italic text-muted-foreground">
							{t(
								"nothingToSummarizeYet",
								"Nothing to summarize yet — load some objects first.",
							)}
						</p>
					)}
					{facets.map((facet) => {
						const maxCount = facet.values.reduce(
							(max, value) => Math.max(max, value.count),
							1,
						);
						return (
							<div key={facet.key} className="space-y-1">
								<p className="truncate text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									{facet.title}
								</p>
								{facet.values.map((value) => {
									const isSelected = selected.has(
										selectionKey(facet.key, value.key),
									);
									return (
										<button
											type="button"
											key={value.key}
											className={`relative flex w-full items-center gap-2 overflow-hidden rounded px-2 py-1 text-left text-xs transition-colors ${
												isSelected ? "bg-primary/15" : "hover:bg-accent"
											}`}
											onClick={() => toggle(facet.key, value.key)}
											onMouseEnter={() => onHoverValue(new Set(value.ids))}
											onMouseLeave={() => onHoverValue(null)}
										>
											<span
												className="absolute inset-y-0 left-0 bg-primary/10"
												style={{
													width: `${Math.round((value.count / maxCount) * 100)}%`,
												}}
											/>
											{isSelected && (
												<Check className="relative h-3 w-3 shrink-0 text-primary" />
											)}
											<span className="relative min-w-0 flex-1 truncate">
												{value.label}
											</span>
											<span className="relative shrink-0 tabular-nums text-muted-foreground">
												{value.count.toLocaleString()}
											</span>
										</button>
									);
								})}
							</div>
						);
					})}
				</div>
			</ScrollArea>

			<div className="shrink-0 space-y-1.5 border-t p-3">
				<div className="flex gap-1.5">
					<Button
						variant="outline"
						size="sm"
						className="h-7 flex-1 gap-1.5 text-xs"
						disabled={!selectedIds || selectedIds.ids.size === 0}
						onClick={() => applyFilter("to")}
					>
						<Filter className="h-3.5 w-3.5" />
						{t("filterTo", "Filter to")}
					</Button>
					<Button
						variant="outline"
						size="sm"
						className="h-7 flex-1 gap-1.5 text-xs"
						disabled={!selectedIds || selectedIds.ids.size === 0}
						onClick={() => applyFilter("out")}
					>
						<FilterX className="h-3.5 w-3.5" />
						{t("exclude", "Exclude")}
					</Button>
				</div>
				<p className="text-[10px] text-muted-foreground">
					{selectedIds
						? t("countObjectsSelected", "{{count}} objects selected", {
								count: selectedIds.ids.size,
							})
						: t(
								"selectBarsToFilterTheCanvas",
								"Select bars to filter the canvas; hover to highlight.",
							)}
				</p>
			</div>
		</div>
	);
}
