"use client";

import { useTranslation } from "@flow-like/locales";
import { Group, Ungroup } from "lucide-react";
import { Button } from "../button";
import { Label } from "../label";
import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import { Slider } from "../slider";

/** Degrees the leaf cutoff offers. Beyond 2 it stops being "trim the fringe". */
const MAX_LEAF_CUTOFF = 2;

export interface GraphDensityControlProps {
	collapsedGroups: number;
	groupCount: number;
	onCollapseAll: () => void;
	onExpandAll: () => void;
	leafCutoff: number;
	onLeafCutoffChange: (cutoff: number) => void;
	hiddenLeaves: number;
}

/**
 * The two levers that decide how much graph is on screen, kept together because
 * they answer the same question and are otherwise easy to forget are on.
 */
export function GraphDensityControl({
	collapsedGroups,
	groupCount,
	onCollapseAll,
	onExpandAll,
	leafCutoff,
	onLeafCutoffChange,
	hiddenLeaves,
}: GraphDensityControlProps) {
	const { t } = useTranslation("common");
	const active = collapsedGroups > 0 || leafCutoff > 0;
	const canGroup = groupCount > 1;

	if (!canGroup && hiddenLeaves === 0 && leafCutoff === 0) return null;

	return (
		<Popover>
			<PopoverTrigger asChild>
				<button
					type="button"
					className={`flex items-center gap-1.5 whitespace-nowrap rounded border px-2 py-1 text-xs transition-colors ${
						active
							? "border-primary/40 bg-primary/10 text-foreground"
							: "text-muted-foreground hover:text-foreground"
					}`}
					title={t(
						"controlHowMuchOfTheGraphIsDrawn",
						"Control how much of the graph is drawn",
					)}
				>
					<Group className="h-3.5 w-3.5" />
					{active ? t("simplifiedOn", "Simplified") : t("simplify", "Simplify")}
				</button>
			</PopoverTrigger>
			<PopoverContent align="start" className="w-72 space-y-4">
				{canGroup && (
					<div className="space-y-2">
						<div className="flex items-center justify-between">
							<Label className="text-xs">{t("groups", "Groups")}</Label>
							<span className="text-[10px] tabular-nums text-muted-foreground">
								{t(
									"countOfTotalCollapsed",
									"{{count}} of {{total}} collapsed",
									{
										count: collapsedGroups,
										total: groupCount,
									},
								)}
							</span>
						</div>
						<div className="flex gap-1.5">
							<Button
								variant="outline"
								size="sm"
								className="h-7 flex-1 gap-1.5 text-xs"
								onClick={onCollapseAll}
								disabled={collapsedGroups >= groupCount}
							>
								<Group className="h-3.5 w-3.5" />
								{t("collapseAll", "Collapse all")}
							</Button>
							<Button
								variant="outline"
								size="sm"
								className="h-7 flex-1 gap-1.5 text-xs"
								onClick={onExpandAll}
								disabled={collapsedGroups === 0}
							>
								<Ungroup className="h-3.5 w-3.5" />
								{t("expandAll", "Expand all")}
							</Button>
						</div>
						<p className="text-[11px] text-muted-foreground">
							{t(
								"aCollapsedGroupIsOneObjectStandingForItsMembersClickItToOpen",
								"A collapsed group is one object standing for its members. Click it on the canvas to open it.",
							)}
						</p>
					</div>
				)}

				<div className="space-y-2">
					<div className="flex items-center justify-between">
						<Label className="text-xs">{t("hideLeaves", "Hide leaves")}</Label>
						<span className="text-[10px] tabular-nums text-muted-foreground">
							{leafCutoff === 0
								? t("off", "Off")
								: t("countHidden", "{{count}} hidden", { count: hiddenLeaves })}
						</span>
					</div>
					<Slider
						value={[leafCutoff]}
						min={0}
						max={MAX_LEAF_CUTOFF}
						step={1}
						onValueChange={([next]) => onLeafCutoffChange(next ?? 0)}
					/>
					<p className="text-[11px] text-muted-foreground">
						{leafCutoff === 0
							? t("everyLoadedObjectIsDrawn", "Every loaded object is drawn.")
							: t(
									"hidesObjectsWithCountOrFewerConnectionsInThisView",
									"Hides objects with {{count}} or fewer connections in this view, leaving the backbone.",
									{ count: leafCutoff },
								)}
					</p>
				</div>
			</PopoverContent>
		</Popover>
	);
}
