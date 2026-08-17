"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftRight, ArrowRight, Expand } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type {
	GraphOverlay,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import { Button } from "../button";
import { Checkbox } from "../checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../dialog";
import { Label } from "../label";
import { Slider } from "../slider";
import { getGraphIcon } from "./icons";

export type ExpansionDirection = "outgoing" | "incoming" | "both";

export interface ExpansionOptions {
	edgeLabels?: string[];
	direction?: ExpansionDirection;
	limit?: number;
}

export interface ExpansionChoice {
	label: string;
	/** Whole-population fan-out along this relationship, when the sampler knows it. */
	total?: number;
	exact: boolean;
	/** How many of this relationship's neighbors are already on the stage. */
	loaded: number;
	direction: "outgoing" | "incoming";
	otherLabel: string;
	color: string;
	icon: string;
}

export interface GraphExpansionDialogProps {
	node: SubgraphNode | null;
	overlay: GraphOverlay;
	choices: ExpansionChoice[];
	maxLimit: number;
	onClose: () => void;
	onExpand: (options: ExpansionOptions) => void;
}

const LIMIT_STEPS = [10, 25, 50, 100, 250, 500] as const;
const DEFAULT_LIMIT_INDEX = 2;

function directionOf(choices: readonly ExpansionChoice[]): ExpansionDirection {
	const hasOut = choices.some((choice) => choice.direction === "outgoing");
	const hasIn = choices.some((choice) => choice.direction === "incoming");
	if (hasOut && hasIn) return "both";
	return hasIn ? "incoming" : "outgoing";
}

/**
 * Asks before pulling neighbors in.
 *
 * An unguarded expand is what rebuilds the hairball a reader just escaped: one
 * object here can hang off two hundred movements, and blind expansion spends its
 * whole budget on whichever relationship happens to come back first. Naming the
 * relationship and the ceiling up front is the difference between exploring a
 * graph and detonating one.
 */
export function GraphExpansionDialog({
	node,
	choices,
	maxLimit,
	onClose,
	onExpand,
}: GraphExpansionDialogProps) {
	const { t } = useTranslation("common");
	const [selected, setSelected] = useState<Set<string>>(new Set());
	const [limitIndex, setLimitIndex] = useState(DEFAULT_LIMIT_INDEX);

	// Every relationship starts checked: the dialog is a brake, not a form to fill
	// in, so confirming without touching anything must do the obvious thing.
	useEffect(() => {
		if (!node) return;
		setSelected(new Set(choices.map((choice) => choice.label)));
		setLimitIndex(DEFAULT_LIMIT_INDEX);
	}, [node, choices]);

	const limit = Math.min(LIMIT_STEPS[limitIndex] ?? 50, maxLimit);

	const selectedChoices = useMemo(
		() => choices.filter((choice) => selected.has(choice.label)),
		[choices, selected],
	);

	const expected = useMemo(() => {
		let known = 0;
		let unknown = false;
		for (const choice of selectedChoices) {
			if (choice.total === undefined) unknown = true;
			else known += choice.total;
		}
		return { known, unknown };
	}, [selectedChoices]);

	if (!node) return null;

	const toggle = (label: string) => {
		setSelected((prev) => {
			const next = new Set(prev);
			if (next.has(label)) next.delete(label);
			else next.add(label);
			return next;
		});
	};

	const confirm = () => {
		onExpand({
			edgeLabels: selectedChoices.map((choice) => choice.label),
			direction: directionOf(selectedChoices),
			limit,
		});
		onClose();
	};

	return (
		<Dialog open={node !== null} onOpenChange={(open) => !open && onClose()}>
			{/* A column with one flexing row: the relationship list is the only part
			    allowed to grow, so the limit slider and the buttons can never be
			    pushed off the bottom by an ontology with thirty mappings. */}
			<DialogContent className="flex max-h-[85vh] flex-col gap-4 sm:max-w-lg">
				<DialogHeader className="shrink-0">
					<DialogTitle className="flex items-center gap-2">
						<Expand className="h-4 w-4" />
						{t("expandFromName", "Expand from {{name}}", {
							name: node.caption ?? node.id,
						})}
					</DialogTitle>
					<DialogDescription>
						{t(
							"chooseWhichRelationshipsToFollowAndHowManyObjectsToBringIn",
							"Choose which relationships to follow and how many objects to bring in.",
						)}
					</DialogDescription>
				</DialogHeader>

				{choices.length === 0 ? (
					<p className="py-4 text-sm text-muted-foreground">
						{t(
							"thisObjectHasNoMappedRelationshipsToFollow",
							"This object has no mapped relationships to follow.",
						)}
					</p>
				) : (
					<div className="min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
						{choices.map((choice) => {
							const Icon = getGraphIcon(choice.icon);
							const isSelected = selected.has(choice.label);
							return (
								<button
									type="button"
									key={`${choice.direction}-${choice.label}`}
									className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-accent"
									onClick={() => toggle(choice.label)}
								>
									<Checkbox checked={isSelected} className="shrink-0" />
									{choice.direction === "outgoing" ? (
										<ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
									) : (
										<ArrowLeftRight className="h-3 w-3 shrink-0 text-muted-foreground" />
									)}
									<span className="min-w-0 flex-1 truncate font-mono text-xs">
										{choice.label}
									</span>
									<span
										className="flex w-28 shrink-0 items-center gap-1 text-[10px] text-muted-foreground"
										title={choice.otherLabel}
									>
										<span
											className="flex h-3 w-3 shrink-0 items-center justify-center rounded-full"
											style={{ backgroundColor: choice.color }}
										>
											<Icon className="h-2 w-2 text-white" />
										</span>
										<span className="truncate">{choice.otherLabel}</span>
									</span>
									<span className="w-10 shrink-0 text-right tabular-nums text-xs text-muted-foreground">
										{choice.total === undefined
											? "—"
											: `${choice.exact ? "" : "≥"}${choice.total.toLocaleString()}`}
									</span>
								</button>
							);
						})}
					</div>
				)}

				<div className="shrink-0 space-y-2 border-t pt-3">
					<div className="flex items-center justify-between">
						<Label className="text-xs">
							{t("mostObjectsToAdd", "Most objects to add")}
						</Label>
						<span className="tabular-nums text-xs font-medium">
							{limit.toLocaleString()}
						</span>
					</div>
					<Slider
						value={[limitIndex]}
						min={0}
						max={LIMIT_STEPS.length - 1}
						step={1}
						onValueChange={([next]) => setLimitIndex(next ?? 0)}
					/>
					<p className="text-[11px] text-muted-foreground">
						{expected.known === 0 && !expected.unknown
							? t(
									"selectAtLeastOneRelationship",
									"Select at least one relationship.",
								)
							: t(
									"thisObjectHasAboutAmountLinkedAlongTheSelected",
									"This object has about {{amount}} linked objects along the selected relationships.",
									{
										amount: `${expected.unknown ? "≥" : ""}${expected.known.toLocaleString()}`,
									},
								)}
					</p>
				</div>

				<DialogFooter className="shrink-0">
					<Button variant="outline" onClick={onClose}>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={confirm} disabled={selectedChoices.length === 0}>
						{t("expand", "Expand")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

/**
 * What a node can be expanded along, from the overlay's mappings plus whatever
 * the sampler measured for this object.
 */
export function buildExpansionChoices(
	node: SubgraphNode,
	overlay: GraphOverlay,
	loadedByLabel: ReadonlyMap<string, number>,
): ExpansionChoice[] {
	const statsByLabel = new Map(
		(node.stats?.out_by_label ?? []).map((entry) => [entry.label, entry.count]),
	);
	const styleByLabel = new Map(
		overlay.nodes.map((mapping) => [mapping.label, mapping.style]),
	);

	const choices: ExpansionChoice[] = [];
	for (const edge of overlay.edges) {
		const outgoing = edge.src_label === node.label;
		const incoming = edge.dst_label === node.label;
		if (!outgoing && !incoming) continue;

		const otherLabel = outgoing ? edge.dst_label : edge.src_label;
		const style = styleByLabel.get(otherLabel);
		choices.push({
			label: edge.label,
			total: statsByLabel.get(edge.label),
			exact: node.stats?.exact ?? true,
			loaded: loadedByLabel.get(edge.label) ?? 0,
			direction: outgoing ? "outgoing" : "incoming",
			otherLabel,
			color: style?.color ?? "#64748b",
			icon: style?.icon ?? "database",
		});
	}

	// Biggest fan-out first — that is the one a reader needs to think about before
	// confirming, and the one an unguarded expand would have detonated.
	return choices.sort(
		(a, b) => (b.total ?? 0) - (a.total ?? 0) || a.label.localeCompare(b.label),
	);
}
