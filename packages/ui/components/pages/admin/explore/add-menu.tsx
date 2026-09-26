"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import type {
	ExploreLayoutDoc,
	ExploreSlotKey,
} from "../../../store/explore/explore-types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import {
	type AddChoice,
	type AddTarget,
	choiceLabel,
	findSlot,
	slotLabel,
	slotSize,
} from "./explore-admin-model";
import { KindIcon } from "./explore-admin-visuals";

/** A slot's label plus the name of its first placement, so rows are recognisable in menus. */
export function SlotName({
	layout,
	slot,
}: {
	layout: ExploreLayoutDoc;
	slot: ExploreSlotKey;
}) {
	const { t } = useTranslation("admin");
	const first = findSlot(layout, slot)?.placements[0]?.name;
	return (
		<span className="flex min-w-0 flex-1 flex-col">
			<span className="truncate">
				{slot === "unplaced"
					? t("exploreAddUnplaced", "Unplaced (not on the page)")
					: slotLabel(layout, slot, t)}
			</span>
			{first && slot !== "unplaced" && (
				<span className="truncate text-xs text-muted-foreground">{first}</span>
			)}
		</span>
	);
}

export type AddHandler = (target: AddTarget, choice: AddChoice) => void;

function TargetLabel({
	layout,
	target,
}: {
	layout: ExploreLayoutDoc;
	target: AddTarget;
}) {
	return (
		<>
			<SlotName layout={layout} slot={target.slot} />
			{target.slot !== "unplaced" && (
				<span className="font-mono text-[11px] text-muted-foreground">
					{slotSize(target.slot)}
				</span>
			)}
		</>
	);
}

/** Only compatible slots are listed: `addTargets` already filtered them through `slotAccepts`. */
export function AddMenu({
	layout,
	targets,
	onAdd,
	children,
	align = "start",
}: {
	layout: ExploreLayoutDoc;
	targets: readonly AddTarget[];
	onAdd: AddHandler;
	children: ReactNode;
	align?: "start" | "end" | "center";
}) {
	const { t } = useTranslation("admin");
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
			<DropdownMenuContent align={align} className="w-60">
				<DropdownMenuLabel className="text-xs font-medium text-muted-foreground">
					{t("exploreAddTo", "Add to")}
				</DropdownMenuLabel>
				{targets.map((target) =>
					target.choices.length === 1 ? (
						<DropdownMenuItem
							key={target.slot}
							onSelect={() => onAdd(target, target.choices[0])}
						>
							<TargetLabel layout={layout} target={target} />
						</DropdownMenuItem>
					) : (
						<DropdownMenuSub key={target.slot}>
							<DropdownMenuSubTrigger>
								<TargetLabel layout={layout} target={target} />
							</DropdownMenuSubTrigger>
							<DropdownMenuSubContent className="max-h-80 overflow-y-auto">
								{target.choices.map((choice) => (
									<DropdownMenuItem
										key={`${choice.kind}:${choice.rail ?? ""}`}
										onSelect={() => onAdd(target, choice)}
									>
										<KindIcon kind={choice.kind} />
										{choiceLabel(choice, t)}
									</DropdownMenuItem>
								))}
							</DropdownMenuSubContent>
						</DropdownMenuSub>
					),
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}
