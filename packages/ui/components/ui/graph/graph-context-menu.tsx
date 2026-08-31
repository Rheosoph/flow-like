"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowLeftRight,
	ArrowRight,
	Copy,
	Crosshair,
	Expand,
	EyeOff,
	Group,
	Pin,
	PinOff,
	Route,
	SlidersHorizontal,
	Ungroup,
} from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import type { SubgraphNode } from "../../../state/backend-state/graph-state";
import type { ExpansionChoice } from "./graph-expansion-dialog";
import { getGraphIcon } from "./icons";

export interface GraphContextMenuState {
	nodeId: string;
	x: number;
	y: number;
}

export interface GraphContextMenuProps {
	state: GraphContextMenuState | null;
	node: SubgraphNode | null;
	isGroup: boolean;
	/** Cluster the object belongs to, when that cluster can be collapsed. */
	collapsibleClusterId?: string | null;
	pinned: boolean;
	focused: boolean;
	choices: ExpansionChoice[];
	onClose: () => void;
	onExpandChoice?: (choice: ExpansionChoice) => void;
	onExpandAll?: () => void;
	onGuidedExpand?: () => void;
	onOpenGroup?: () => void;
	onCollapseGroup?: () => void;
	onToggleFocus?: () => void;
	onHide?: () => void;
	onTogglePin?: () => void;
	onFindPath?: () => void;
}

/** Relationships listed inline before the menu defers to the guided dialog. */
const MAX_INLINE_CHOICES = 6;

function MenuItem({
	icon,
	label,
	onClick,
}: {
	icon: React.ReactNode;
	label: string;
	onClick: () => void;
}) {
	return (
		<button
			type="button"
			className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-accent transition-colors"
			onClick={onClick}
		>
			{icon}
			<span className="min-w-0 flex-1 truncate">{label}</span>
		</button>
	);
}

/**
 * The right-click surface for a node: scoped expansion with the counts that
 * make it a decision instead of a gamble, plus the object-level verbs.
 */
export function GraphContextMenu({
	state,
	node,
	isGroup,
	collapsibleClusterId,
	pinned,
	focused,
	choices,
	onClose,
	onExpandChoice,
	onExpandAll,
	onGuidedExpand,
	onOpenGroup,
	onCollapseGroup,
	onToggleFocus,
	onHide,
	onTogglePin,
	onFindPath,
}: GraphContextMenuProps) {
	const { t } = useTranslation("common");
	const panelRef = useRef<HTMLDivElement>(null);
	const [offset, setOffset] = useState<{ x: number; y: number }>({
		x: 0,
		y: 0,
	});

	// Clamp into the canvas so a right-click near an edge never opens off-stage.
	useLayoutEffect(() => {
		setOffset({ x: 0, y: 0 });
		const panel = panelRef.current;
		if (!panel || !state) return;
		const parent = panel.offsetParent as HTMLElement | null;
		if (!parent) return;
		const overflowX = state.x + panel.offsetWidth + 8 - parent.clientWidth;
		const overflowY = state.y + panel.offsetHeight + 8 - parent.clientHeight;
		setOffset({
			x: overflowX > 0 ? -Math.min(overflowX, state.x) : 0,
			y: overflowY > 0 ? -Math.min(overflowY, state.y) : 0,
		});
	}, [state]);

	if (!state) return null;

	const inlineChoices = choices.slice(0, MAX_INLINE_CHOICES);
	const iconClass = "h-3.5 w-3.5 shrink-0 text-muted-foreground";
	const handle = (action?: () => void) => () => {
		action?.();
		onClose();
	};

	return (
		<>
			{/* Click-away layer; contextmenu on it closes without opening a new menu. */}
			<div
				className="absolute inset-0 z-30"
				onMouseDown={onClose}
				onContextMenu={(event) => {
					event.preventDefault();
					onClose();
				}}
			/>
			<div
				ref={panelRef}
				className="absolute z-40 w-60 rounded-lg border bg-popover p-1 shadow-lg"
				style={{ left: state.x + offset.x, top: state.y + offset.y }}
				role="menu"
			>
				<div className="px-2 py-1.5">
					<p className="truncate text-xs font-medium">
						{node?.caption ?? state.nodeId}
					</p>
					<p className="truncate text-[10px] text-muted-foreground">
						{isGroup
							? t("collapsedGroup", "Collapsed group")
							: (node?.label ?? "")}
					</p>
				</div>
				<div className="my-1 h-px bg-border" />

				{isGroup && onOpenGroup && (
					<MenuItem
						icon={<Ungroup className={iconClass} />}
						label={t("openGroup", "Open group")}
						onClick={handle(onOpenGroup)}
					/>
				)}

				{!isGroup && inlineChoices.length > 0 && onExpandChoice && (
					<>
						<p className="px-2 pt-1 pb-0.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
							{t("expandAlong", "Expand along")}
						</p>
						{inlineChoices.map((choice) => {
							const Icon = getGraphIcon(choice.icon);
							return (
								<button
									type="button"
									key={`${choice.direction}-${choice.label}`}
									className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-accent transition-colors"
									onClick={handle(() => onExpandChoice(choice))}
								>
									{choice.direction === "outgoing" ? (
										<ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
									) : (
										<ArrowLeftRight className="h-3 w-3 shrink-0 text-muted-foreground" />
									)}
									<span className="min-w-0 flex-1 truncate font-mono">
										{choice.label}
									</span>
									<span
										className="flex h-3 w-3 shrink-0 items-center justify-center rounded-full"
										style={{ backgroundColor: choice.color }}
									>
										<Icon className="h-2 w-2 text-white" />
									</span>
									<span className="shrink-0 tabular-nums text-[10px] text-muted-foreground">
										{choice.total === undefined
											? "—"
											: `${choice.exact ? "" : "≥"}${choice.total.toLocaleString()}`}
									</span>
								</button>
							);
						})}
					</>
				)}

				{!isGroup && onExpandAll && (
					<MenuItem
						icon={<Expand className={iconClass} />}
						label={t("expandAllRelationships", "Expand all relationships")}
						onClick={handle(onExpandAll)}
					/>
				)}
				{!isGroup && onGuidedExpand && (
					<MenuItem
						icon={<SlidersHorizontal className={iconClass} />}
						label={t("expandWith", "Expand with…")}
						onClick={handle(onGuidedExpand)}
					/>
				)}

				{(onToggleFocus || onHide || onTogglePin || onFindPath) && (
					<div className="my-1 h-px bg-border" />
				)}

				{!isGroup && onToggleFocus && (
					<MenuItem
						icon={<Crosshair className={iconClass} />}
						label={focused ? t("exitFocus", "Exit focus") : t("focus", "Focus")}
						onClick={handle(onToggleFocus)}
					/>
				)}
				{onTogglePin && (
					<MenuItem
						icon={
							pinned ? (
								<PinOff className={iconClass} />
							) : (
								<Pin className={iconClass} />
							)
						}
						label={
							pinned ? t("unpinNode", "Unpin") : t("pinNode", "Pin in place")
						}
						onClick={handle(onTogglePin)}
					/>
				)}
				{onHide && (
					<MenuItem
						icon={<EyeOff className={iconClass} />}
						label={t("hideObject", "Hide from view")}
						onClick={handle(onHide)}
					/>
				)}
				{!isGroup && collapsibleClusterId && onCollapseGroup && (
					<MenuItem
						icon={<Group className={iconClass} />}
						label={t("collapseItsGroup", "Collapse its group")}
						onClick={handle(onCollapseGroup)}
					/>
				)}
				{!isGroup && onFindPath && (
					<MenuItem
						icon={<Route className={iconClass} />}
						label={t("findPathFromHere", "Find path from here")}
						onClick={handle(onFindPath)}
					/>
				)}

				<div className="my-1 h-px bg-border" />
				<MenuItem
					icon={<Copy className={iconClass} />}
					label={t("copyId", "Copy ID")}
					onClick={handle(() => {
						void navigator.clipboard?.writeText(node?.id ?? state.nodeId);
					})}
				/>
			</div>
		</>
	);
}
