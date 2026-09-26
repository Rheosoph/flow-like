"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Position,
	ViewportPortal,
	getBezierPath,
	useInternalNode,
} from "@xyflow/react";
import {
	ChevronLeftIcon,
	ChevronRightIcon,
	WorkflowIcon,
	XIcon,
} from "lucide-react";
import {
	type MouseEvent as ReactMouseEvent,
	type ReactNode,
	type PointerEvent as ReactPointerEvent,
	memo,
	useMemo,
} from "react";
import {
	type NodeSuggestionGhostView,
	ghostPlacement,
	ghostTargetPin,
} from "../../hooks/use-node-suggestion-ghost";
import { DynamicImage } from "../ui";
import { typeToColor } from "./utils";

export interface NodeSuggestionGhostProps {
	view: NodeSuggestionGhostView;
	onAccept: () => void;
	onDismiss: () => void;
	onCycle: (step: 1 | -1) => void;
}

const stopPointer = (event: ReactPointerEvent) => event.stopPropagation();

function GhostIconButton({
	label,
	disabled,
	onClick,
	children,
}: {
	label: string;
	disabled?: boolean;
	onClick: () => void;
	children: ReactNode;
}) {
	return (
		<button
			type="button"
			aria-label={label}
			title={label}
			disabled={disabled}
			className="nodrag nopan pointer-events-auto flex size-3.5 items-center justify-center rounded-sm text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
			onPointerDown={stopPointer}
			onClick={(event: ReactMouseEvent) => {
				event.stopPropagation();
				onClick();
			}}
		>
			{children}
		</button>
	);
}

export const NodeSuggestionGhost = memo(function NodeSuggestionGhost({
	view,
	onAccept,
	onDismiss,
	onCycle,
}: NodeSuggestionGhostProps) {
	const { t } = useTranslation("flow");
	const anchor = useInternalNode(view.anchorNodeId);
	const { ghost, anchorPin } = view;
	const placement = useMemo(
		() =>
			anchor && !anchor.dragging
				? ghostPlacement(anchor, anchorPin.id, ghost)
				: undefined,
		[anchor, anchorPin.id, ghost],
	);
	const targetPin = useMemo(() => ghostTargetPin(ghost), [ghost]);
	if (!placement) return null;

	const { source, target, origin, size, targetOffset } = placement;
	const [path] = getBezierPath({
		sourceX: source.x,
		sourceY: source.y,
		sourcePosition: Position.Right,
		targetX: target.x,
		targetY: target.y,
		targetPosition: Position.Left,
	});
	const name = ghost.node.friendly_name || ghost.node.name;
	const wireColor = typeToColor(anchorPin.data_type);

	return (
		<ViewportPortal>
			<div className="pointer-events-none" data-node-suggestion-ghost="">
				<svg
					width="1"
					height="1"
					className="pointer-events-none absolute overflow-visible"
					aria-hidden="true"
				>
					<path
						d={path}
						fill="none"
						stroke={wireColor}
						strokeWidth={1.5}
						strokeDasharray="5 4"
						opacity={0.7}
					/>
				</svg>
				<div
					className="group/ghost absolute"
					style={{ left: origin.x, top: origin.y, width: size.width }}
				>
					<button
						type="button"
						disabled={view.busy}
						aria-label={t(
							"addSuggestedNodeName",
							"Add suggested node {{name}}",
							{ name },
						)}
						title={ghost.node.description || name}
						className="nodrag nopan pointer-events-auto relative flex w-full flex-col justify-start rounded-md border border-dashed border-muted-foreground/60 bg-card/50 text-left opacity-60 shadow-sm transition-opacity hover:opacity-100 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-progress group-hover/ghost:opacity-100"
						style={{ height: size.height }}
						onPointerDown={stopPointer}
						onClick={(event) => {
							event.stopPropagation();
							onAccept();
						}}
					>
						<span className="flex h-4 min-w-0 items-center gap-1 border-b border-dashed border-muted-foreground/40 px-1">
							{ghost.node.icon ? (
								<DynamicImage
									className="size-2 shrink-0 bg-foreground"
									url={ghost.node.icon}
								/>
							) : (
								<WorkflowIcon className="size-2 shrink-0" aria-hidden="true" />
							)}
							<span className="truncate text-[9px] font-medium leading-none">
								{name}
							</span>
						</span>
						{targetPin && (
							<span
								className="absolute left-0 flex max-w-30 -translate-x-0.75 -translate-y-1/2 items-center gap-1 text-[8px] text-muted-foreground"
								style={{ top: targetOffset }}
							>
								<span
									className="size-1.5 shrink-0 rounded-full"
									style={{ backgroundColor: typeToColor(targetPin.data_type) }}
								/>
								<span className="truncate">
									{targetPin.friendly_name || targetPin.name}
								</span>
							</span>
						)}
					</button>
					<div className="pointer-events-none mt-1 flex items-center gap-1 text-[8px] leading-none text-muted-foreground">
						<kbd className="rounded-sm border border-border bg-muted px-1 py-0.5 font-mono text-[7px] leading-none text-foreground">
							{t("tab", "Tab")}
						</kbd>
						<span className="truncate">{t("toAdd", "to add")}</span>
						<span className="ml-auto flex items-center gap-0.5">
							{view.count > 1 && (
								<>
									<GhostIconButton
										label={t("previousSuggestion", "Previous suggestion")}
										disabled={view.busy}
										onClick={() => onCycle(-1)}
									>
										<ChevronLeftIcon className="size-2.5" aria-hidden="true" />
									</GhostIconButton>
									<span className="tabular-nums" aria-hidden="true">
										{view.index + 1}/{view.count}
									</span>
									<span className="sr-only">
										{t(
											"suggestionIndexOfCount",
											"Suggestion {{index}} of {{count}}",
											{ index: view.index + 1, count: view.count },
										)}
									</span>
									<GhostIconButton
										label={t("nextSuggestion", "Next suggestion")}
										disabled={view.busy}
										onClick={() => onCycle(1)}
									>
										<ChevronRightIcon className="size-2.5" aria-hidden="true" />
									</GhostIconButton>
								</>
							)}
							<GhostIconButton
								label={t("dismissSuggestion", "Dismiss suggestion")}
								disabled={view.busy}
								onClick={onDismiss}
							>
								<XIcon className="size-2.5" aria-hidden="true" />
							</GhostIconButton>
						</span>
					</div>
				</div>
			</div>
		</ViewportPortal>
	);
});
