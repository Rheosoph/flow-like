"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Panel,
	Position,
	ViewportPortal,
	getBezierPath,
	useNodes,
	useReactFlow,
	useViewport,
} from "@xyflow/react";
import ChevronLeftIcon from "lucide-react/dist/esm/icons/chevron-left.js";
import ChevronRightIcon from "lucide-react/dist/esm/icons/chevron-right.js";
import CircleCheckIcon from "lucide-react/dist/esm/icons/circle-check.js";
import EyeIcon from "lucide-react/dist/esm/icons/eye.js";
import LocateFixedIcon from "lucide-react/dist/esm/icons/locate-fixed.js";
import XIcon from "lucide-react/dist/esm/icons/x.js";
import {
	type ReactElement,
	memo,
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	GroupReviewScope,
	GroupReviewSummary,
} from "../../hooks/use-group-suggestions";
import type { GroupSuggestion } from "../../lib/flow-grouping";
import {
	ENTITY_MIN_HEIGHT,
	NODE_CHROME_HEIGHT,
	NODE_WIDTH,
	PIN_MARGIN_TOP,
	PIN_ROW_HEIGHT,
} from "../../lib/flow-layout/measure";
import { IVariableType } from "../../lib/schema/flow/variable";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import { typeToColor } from "./utils";

type Point = { x: number; y: number };

export interface GroupSuggestionsOverlayProps {
	suggestions: GroupSuggestion[];
	selectedId?: string;
	preview: boolean;
	busy: boolean;
	scope?: GroupReviewScope;
	summary?: GroupReviewSummary;
	/** The last search found nothing new, so "Find more" would only repeat it. */
	exhausted?: boolean;
	onSelect: (id: string) => void;
	onNext: () => void;
	onPrevious: () => void;
	onPreview: () => void;
	onCollapse: () => void;
	onSkip: () => void;
	onFindMore: () => void;
	onClose: () => void;
	getPinPosition?: (nodeId: string, pinId: string) => Point | undefined;
}

const GROUP_PADDING = 20;
/** Room above the outline for its badge, so a jump never tucks it under the strip. */
const BADGE_ROOM = 72;
const JUMP_PADDING = 64;
const JUMP_MAX_ZOOM = 1.25;
const NO_DECISIONS: GroupReviewSummary = { collapsed: 0, skipped: 0 };
const IGNORED_KEY_TARGETS =
	"input, textarea, select, [contenteditable='true'], .monaco-editor, .react-flow__node, .react-flow__edge";

function Hint({
	label,
	shortcut,
	children,
}: {
	label: string;
	shortcut?: string;
	children: ReactElement;
}) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>{children}</TooltipTrigger>
			<TooltipContent side="bottom" className="flex items-center gap-2">
				{label}
				{shortcut && (
					<kbd className="font-mono text-[10px] opacity-70">{shortcut}</kbd>
				)}
			</TooltipContent>
		</Tooltip>
	);
}

function portColor(dataType: string): string {
	const type = Object.values(IVariableType).find((value) => value === dataType);
	return typeToColor(type ?? IVariableType.Generic);
}

function GroupPreview({
	suggestion,
	getPinPosition,
}: Pick<GroupSuggestionsOverlayProps, "getPinPosition"> & {
	suggestion: GroupSuggestion;
}) {
	const { t } = useTranslation("flow");
	const ports = useMemo(() => {
		let inputs = 0;
		let outputs = 0;
		return suggestion.boundaryPorts.map((port) => ({
			...port,
			row: port.direction === "input" ? inputs++ : outputs++,
		}));
	}, [suggestion.boundaryPorts]);
	const rows = Math.max(1, ...ports.map((port) => port.row + 1));
	const height = Math.max(
		ENTITY_MIN_HEIGHT,
		NODE_CHROME_HEIGHT + rows * PIN_ROW_HEIGHT,
	);
	const { x: left, y: top } = suggestion.anchor;
	return (
		<>
			<svg
				width="1"
				height="1"
				className="pointer-events-none absolute overflow-visible"
				aria-hidden="true"
				data-group-preview-wires=""
			>
				{ports.flatMap((port) => {
					const input = port.direction === "input";
					const ghost = {
						x: left + (input ? 0 : NODE_WIDTH),
						y: top + PIN_MARGIN_TOP + port.row * PIN_ROW_HEIGHT,
					};
					return port.connections.flatMap((connection) => {
						const outside = getPinPosition?.(
							connection.nodeId,
							connection.pinId,
						);
						if (!outside) return [];
						const source = input ? outside : ghost;
						const target = input ? ghost : outside;
						const [path] = getBezierPath({
							sourceX: source.x,
							sourceY: source.y,
							sourcePosition: Position.Right,
							targetX: target.x,
							targetY: target.y,
							targetPosition: Position.Left,
						});
						return [
							<path
								key={`${port.id}:${connection.nodeId}:${connection.pinId}`}
								d={path}
								fill="none"
								stroke={portColor(port.dataType)}
								strokeWidth={1.5}
								strokeDasharray="5 4"
								opacity={0.8}
							/>,
						];
					});
				})}
			</svg>
			<div
				className="pointer-events-none absolute rounded-lg border border-dashed border-primary bg-background/95 shadow-lg"
				style={{ left, top, width: NODE_WIDTH, height }}
				data-group-preview={suggestion.id}
			>
				<p className="absolute -top-5 left-0 text-[9px] uppercase tracking-wide text-muted-foreground">
					{t("groupSuggestionPreview", "Preview")}
				</p>
				<div className="flex h-4 items-center border-b border-border px-1">
					<p className="truncate text-[9px] font-medium leading-none">
						{suggestion.label}
					</p>
				</div>
				{ports.map((port) => {
					const input = port.direction === "input";
					return (
						<div
							key={port.id}
							className={cn(
								"absolute flex max-w-[68px] -translate-y-1/2 items-center gap-1 text-[8px]",
								input
									? "left-0 -translate-x-[3px]"
									: "right-0 translate-x-[3px] flex-row-reverse",
							)}
							style={{ top: PIN_MARGIN_TOP + port.row * PIN_ROW_HEIGHT }}
							title={`${port.label}: ${port.dataType}`}
						>
							<span
								className="size-1.5 shrink-0 rounded-full"
								style={{ backgroundColor: portColor(port.dataType) }}
								data-group-port={port.id}
								data-type={port.dataType}
							/>
							<span className="truncate">{port.label}</span>
						</div>
					);
				})}
			</div>
		</>
	);
}

function ReviewNavigation({
	index,
	count,
	busy,
	onNext,
	onPrevious,
}: {
	index: number;
	count: number;
	busy: boolean;
	onNext: () => void;
	onPrevious: () => void;
}) {
	const { t } = useTranslation("flow");
	return (
		<div className="flex shrink-0 items-center border-r border-border pr-1.5">
			<Hint
				label={t("previousGroupSuggestion", "Previous suggestion")}
				shortcut="←"
			>
				<Button
					type="button"
					size="icon"
					variant="ghost"
					className="size-7"
					aria-label={t("previousGroupSuggestion", "Previous suggestion")}
					disabled={busy}
					onClick={onPrevious}
				>
					<ChevronLeftIcon aria-hidden="true" className="size-4" />
				</Button>
			</Hint>
			<span className="min-w-9 text-center text-xs tabular-nums text-muted-foreground">
				<span aria-hidden="true">
					{index + 1}/{count}
				</span>
				<span className="sr-only">
					{t("groupSuggestionPosition", "Suggestion {{index}} of {{count}}", {
						index: index + 1,
						count,
					})}
				</span>
			</span>
			<Hint label={t("nextGroupSuggestion", "Next suggestion")} shortcut="→">
				<Button
					type="button"
					size="icon"
					variant="ghost"
					className="size-7"
					aria-label={t("nextGroupSuggestion", "Next suggestion")}
					disabled={busy}
					onClick={onNext}
				>
					<ChevronRightIcon aria-hidden="true" className="size-4" />
				</Button>
			</Hint>
		</div>
	);
}

function ReviewTarget({
	suggestion,
	onJump,
}: {
	suggestion: GroupSuggestion;
	onJump: () => void;
}) {
	const { t } = useTranslation("flow");
	return (
		<Hint label={t("showGroupOnBoard", "Show on board")}>
			<button
				type="button"
				className="group flex min-w-0 flex-1 items-center gap-2 rounded-md px-1.5 py-0.5 text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				onClick={onJump}
			>
				<LocateFixedIcon
					aria-hidden="true"
					className="size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground"
				/>
				<span className="min-w-0">
					<span className="block truncate text-sm font-medium">
						{suggestion.label}
					</span>
					<span className="block truncate text-xs text-muted-foreground">
						{t(
							"groupSuggestionCounts",
							"{{nodes}} nodes · {{wires}} internal wires · {{inputs}} in / {{outputs}} out",
							{
								nodes: suggestion.nodeCount,
								wires: suggestion.internalEdgeCount,
								inputs: suggestion.inputCount,
								outputs: suggestion.outputCount,
							},
						)}
					</span>
				</span>
			</button>
		</Hint>
	);
}

function ReviewActions({
	preview,
	busy,
	onPreview,
	onSkip,
	onCollapse,
}: {
	preview: boolean;
	busy: boolean;
	onPreview: () => void;
	onSkip: () => void;
	onCollapse: () => void;
}) {
	const { t } = useTranslation("flow");
	return (
		<div className="flex shrink-0 items-center gap-1.5">
			<Hint
				label={t("previewGroupHint", "Show the collapsed result")}
				shortcut="P"
			>
				<Button
					type="button"
					size="sm"
					variant="outline"
					className={cn(
						preview &&
							"border-primary/70 bg-primary/15 text-primary hover:bg-primary/20 hover:text-primary dark:border-primary/70 dark:bg-primary/15 dark:hover:bg-primary/20",
					)}
					disabled={busy}
					aria-pressed={preview}
					onClick={onPreview}
				>
					<EyeIcon aria-hidden="true" className="size-4" />
					{t("previewGroup", "Preview")}
				</Button>
			</Hint>
			<Hint
				label={t("skipGroupHint", "Not this one — it won't be suggested again")}
				shortcut="S"
			>
				<Button
					type="button"
					size="sm"
					variant="ghost"
					disabled={busy}
					onClick={onSkip}
				>
					{t("skipGroup", "Skip")}
				</Button>
			</Hint>
			<Hint
				label={t("collapseGroupHint", "Collapse into a layer")}
				shortcut="↵"
			>
				<Button type="button" size="sm" disabled={busy} onClick={onCollapse}>
					{busy
						? t("collapsingGroup", "Collapsing…")
						: t("collapseGroup", "Collapse")}
				</Button>
			</Hint>
		</div>
	);
}

function ReviewOutcome({
	scope,
	summary,
	exhausted,
}: {
	scope: GroupReviewScope;
	summary: GroupReviewSummary;
	exhausted: boolean;
}) {
	const { t } = useTranslation("flow");
	const decided = summary.collapsed + summary.skipped > 0;
	const tally = t(
		"groupReviewSummary",
		"{{collapsed}} collapsed · {{skipped}} skipped",
		{ collapsed: summary.collapsed, skipped: summary.skipped },
	);
	const [title, detail] = decided
		? [
				exhausted
					? t("noMoreGroupSuggestions", "No more groups to suggest")
					: t("groupReviewComplete", "All suggestions reviewed"),
				tally,
			]
		: exhausted
			? [
					t("groupSuggestions", "Group suggestions"),
					scope === "selection"
						? t(
								"noGroupSuggestionsInSelection",
								"No groups to suggest in the selection.",
							)
						: t("noGroupSuggestions", "No groups to suggest in this layer."),
				]
			: [
					t("groupSuggestionsOutdated", "Suggestions no longer apply"),
					t(
						"groupSuggestionsOutdatedDetail",
						"The board changed since they were found.",
					),
				];
	return (
		<div className="flex min-w-0 flex-1 items-center gap-2 px-1.5">
			{decided && (
				<CircleCheckIcon
					aria-hidden="true"
					className="size-4 shrink-0 text-primary"
				/>
			)}
			<span className="min-w-0">
				<span className="block truncate text-sm font-medium">{title}</span>
				<span className="block truncate text-xs text-muted-foreground">
					{detail}
				</span>
			</span>
		</div>
	);
}

export const GroupSuggestionsOverlay = memo(function GroupSuggestionsOverlay({
	suggestions,
	selectedId,
	preview,
	busy,
	scope = "layer",
	summary = NO_DECISIONS,
	exhausted = true,
	onSelect,
	onNext,
	onPrevious,
	onPreview,
	onCollapse,
	onSkip,
	onFindMore,
	onClose,
	getPinPosition,
}: GroupSuggestionsOverlayProps) {
	const { t } = useTranslation("flow");
	const stripRef = useRef<HTMLElement>(null);
	const nodes = useNodes();
	const { getInternalNode, fitView } = useReactFlow();
	const viewport = useViewport();
	const labelRefs = useRef(new Map<string, HTMLButtonElement>());
	const [bottomLabels, setBottomLabels] = useState<ReadonlySet<string>>(
		new Set(),
	);
	const visible = useMemo(() => {
		const renderedNodes = new Map(nodes.map((node) => [node.id, node]));
		return suggestions.slice(0, 3).map((suggestion) => {
			const memberPositions: Point[] = [];
			const boxes = suggestion.memberIds.flatMap((id) => {
				const node = renderedNodes.get(id);
				if (!node) return [];
				const internal = getInternalNode(id);
				const width =
					internal?.measured.width ?? node.measured?.width ?? node.width;
				const height =
					internal?.measured.height ?? node.measured?.height ?? node.height;
				const position = internal?.internals.positionAbsolute ?? node.position;
				memberPositions.push(position);
				if (!width || !height) return [];
				return [{ x: position.x, y: position.y, width, height }];
			});
			const anchor =
				memberPositions.length === suggestion.memberIds.length &&
				memberPositions.length > 0
					? {
							x:
								memberPositions.reduce((sum, point) => sum + point.x, 0) /
								memberPositions.length,
							y:
								memberPositions.reduce((sum, point) => sum + point.y, 0) /
								memberPositions.length,
						}
					: suggestion.anchor;
			if (boxes.length === 0) return { ...suggestion, anchor };
			const left = Math.min(...boxes.map((box) => box.x)) - GROUP_PADDING;
			const top = Math.min(...boxes.map((box) => box.y)) - GROUP_PADDING;
			const right =
				Math.max(...boxes.map((box) => box.x + box.width)) + GROUP_PADDING;
			const bottom =
				Math.max(...boxes.map((box) => box.y + box.height)) + GROUP_PADDING;
			return {
				...suggestion,
				anchor,
				bounds: { x: left, y: top, width: right - left, height: bottom - top },
			};
		});
	}, [suggestions, nodes, getInternalNode]);
	const selectedIndex = Math.max(
		0,
		visible.findIndex((suggestion) => suggestion.id === selectedId),
	);
	const selected = visible[selectedIndex];

	const jump = useCallback(
		(memberIds: readonly string[]) => {
			const strip = stripRef.current;
			const canvas = strip?.closest(".react-flow");
			const covered =
				strip && canvas
					? Math.max(
							0,
							strip.getBoundingClientRect().bottom -
								canvas.getBoundingClientRect().top,
						)
					: 0;
			const reduceMotion = strip?.ownerDocument.defaultView?.matchMedia?.(
				"(prefers-reduced-motion: reduce)",
			).matches;
			void fitView({
				nodes: memberIds.map((id) => ({ id })),
				padding: {
					top: `${Math.round(covered + BADGE_ROOM)}px`,
					right: `${JUMP_PADDING}px`,
					bottom: `${JUMP_PADDING}px`,
					left: `${JUMP_PADDING}px`,
				},
				maxZoom: JUMP_MAX_ZOOM,
				duration: reduceMotion ? 0 : 350,
			});
		},
		[fitView],
	);

	useLayoutEffect(() => {
		const strip = stripRef.current;
		const canvas = strip?.closest(".react-flow");
		if (!strip || !canvas) return;
		const placeLabels = () => {
			const canvasBounds = canvas.getBoundingClientRect();
			const review = strip.getBoundingClientRect();
			const below = new Set<string>();
			for (const suggestion of visible) {
				const label = labelRefs.current.get(suggestion.id);
				if (!label) continue;
				const size = label.getBoundingClientRect();
				const preferred = {
					x:
						canvasBounds.left +
						viewport.x +
						suggestion.bounds.x * viewport.zoom,
					y:
						canvasBounds.top +
						viewport.y +
						(suggestion.bounds.y - 32) * viewport.zoom,
				};
				if (
					size.width > 0 &&
					size.height > 0 &&
					preferred.x < review.right + 8 &&
					preferred.x + size.width > review.left - 8 &&
					preferred.y < review.bottom + 8 &&
					preferred.y + size.height > review.top - 8
				)
					below.add(suggestion.id);
			}
			setBottomLabels((current) =>
				current.size === below.size && [...current].every((id) => below.has(id))
					? current
					: below,
			);
		};
		placeLabels();
		if (typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(placeLabels);
		observer.observe(strip);
		observer.observe(canvas);
		for (const label of labelRefs.current.values()) observer.observe(label);
		return () => observer.disconnect();
	}, [visible, viewport.x, viewport.y, viewport.zoom]);

	useEffect(() => {
		const strip = stripRef.current;
		if (!strip) return;
		const document = strip.ownerDocument;
		const opener = document.activeElement;
		strip.focus({ preventScroll: true });
		return () => {
			const active = document.activeElement;
			if (
				opener instanceof HTMLElement &&
				opener.isConnected &&
				(active === document.body || (active && strip.contains(active)))
			) {
				opener.focus({ preventScroll: true });
			}
		};
	}, []);

	const keys = useRef({
		onClose,
		onNext,
		onPrevious,
		onPreview,
		onCollapse,
		onSkip,
		ready: false,
	});
	keys.current = {
		onClose,
		onNext,
		onPrevious,
		onPreview,
		onCollapse,
		onSkip,
		ready: Boolean(selected) && !busy,
	};

	useEffect(() => {
		const strip = stripRef.current;
		const board = strip?.closest(".react-flow");
		const document = strip?.ownerDocument;
		if (!strip || !document) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (
				event.defaultPrevented ||
				event.isComposing ||
				event.metaKey ||
				event.ctrlKey ||
				event.altKey
			)
				return;
			const target = event.target;
			if (target instanceof Element) {
				if (target.closest('[role="dialog"], [role="alertdialog"]')) return;
				if (
					target !== document.body &&
					!board?.contains(target) &&
					!strip.contains(target)
				)
					return;
			}
			const handlers = keys.current;
			let run: (() => void) | undefined;
			if (event.key === "Escape") run = handlers.onClose;
			else if (
				handlers.ready &&
				!event.repeat &&
				!(target instanceof Element && target.closest(IGNORED_KEY_TARGETS))
			) {
				const onButton =
					target instanceof Element && target.closest("button, a") !== null;
				switch (event.key.toLowerCase()) {
					case "arrowright":
						run = handlers.onNext;
						break;
					case "arrowleft":
						run = handlers.onPrevious;
						break;
					case "p":
						run = handlers.onPreview;
						break;
					case "s":
						run = handlers.onSkip;
						break;
					case "enter":
						if (!onButton) run = handlers.onCollapse;
						break;
				}
			}
			if (!run) return;
			event.preventDefault();
			event.stopPropagation();
			run();
		};
		document.addEventListener("keydown", onKeyDown, true);
		return () => document.removeEventListener("keydown", onKeyDown, true);
	}, []);

	// Keyed by membership, not topology id, so rewiring inside the group never re-jumps.
	const focusKey = selected?.memberIds.join("\0");
	useEffect(() => {
		if (!focusKey) return;
		jump(focusKey.split("\0"));
		const strip = stripRef.current;
		const active = strip?.ownerDocument.activeElement;
		if (strip && (!active || active === strip.ownerDocument.body))
			strip.focus({ preventScroll: true });
	}, [focusKey, jump]);

	return (
		<>
			<ViewportPortal>
				<div className="pointer-events-none" data-group-suggestions="">
					{visible.map((suggestion, index) => {
						const active = suggestion.id === selected?.id;
						return (
							<div
								key={suggestion.id}
								className={cn(
									"pointer-events-none absolute rounded-xl border border-dashed",
									active
										? "border-primary bg-primary/5"
										: "border-muted-foreground/50",
								)}
								style={{
									left: suggestion.bounds.x,
									top: suggestion.bounds.y,
									width: suggestion.bounds.width,
									height: suggestion.bounds.height,
								}}
								data-group-outline={suggestion.id}
							>
								<button
									type="button"
									ref={(element) => {
										if (element) labelRefs.current.set(suggestion.id, element);
										else labelRefs.current.delete(suggestion.id);
									}}
									style={{
										top: bottomLabels.has(suggestion.id)
											? suggestion.bounds.height + 8
											: -32,
									}}
									className={cn(
										"nodrag nopan pointer-events-auto absolute left-0 max-w-64 truncate rounded-md border px-2 py-1 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50",
										active
											? "border-primary bg-primary text-primary-foreground"
											: "border-border bg-background text-foreground",
									)}
									disabled={busy}
									aria-pressed={active}
									aria-label={t(
										"reviewGroupSuggestion",
										"Review {{label}}, {{count}} nodes",
										{ label: suggestion.label, count: suggestion.nodeCount },
									)}
									onPointerDown={(event) => event.stopPropagation()}
									onClick={(event) => {
										event.stopPropagation();
										onSelect(suggestion.id);
									}}
								>
									{index + 1}. {suggestion.label}
								</button>
							</div>
						);
					})}
					{preview && selected && (
						<GroupPreview
							suggestion={selected}
							getPinPosition={getPinPosition}
						/>
					)}
				</div>
			</ViewportPortal>
			<Panel
				position="top-center"
				className="pointer-events-none max-w-[calc(100%-2rem)]"
			>
				<section
					ref={stripRef}
					tabIndex={-1}
					aria-label={t("groupSuggestions", "Group suggestions")}
					aria-busy={busy}
					className="nodrag nopan nowheel pointer-events-auto flex w-2xl max-w-full flex-wrap items-center gap-2 rounded-xl border border-border bg-background/95 px-2 py-1.5 shadow-lg backdrop-blur-sm focus-visible:outline-none"
					onPointerDown={(event) => event.stopPropagation()}
				>
					{visible.length > 1 && (
						<ReviewNavigation
							index={selectedIndex}
							count={visible.length}
							busy={busy}
							onNext={onNext}
							onPrevious={onPrevious}
						/>
					)}
					<div className="flex min-w-0 flex-1 items-center" aria-live="polite">
						{selected ? (
							<ReviewTarget
								suggestion={selected}
								onJump={() => jump(selected.memberIds)}
							/>
						) : (
							<ReviewOutcome
								scope={scope}
								summary={summary}
								exhausted={exhausted}
							/>
						)}
					</div>
					{selected ? (
						<ReviewActions
							preview={preview}
							busy={busy}
							onPreview={onPreview}
							onSkip={onSkip}
							onCollapse={onCollapse}
						/>
					) : (
						!exhausted && (
							<Button
								type="button"
								size="sm"
								variant="outline"
								onClick={onFindMore}
							>
								{t("findMoreGroups", "Find more")}
							</Button>
						)
					)}
					<Hint
						label={t("closeGroupSuggestions", "Close group suggestions")}
						shortcut="Esc"
					>
						<Button
							type="button"
							size="icon"
							variant="ghost"
							className="size-8 shrink-0"
							aria-label={t("closeGroupSuggestions", "Close group suggestions")}
							onClick={onClose}
						>
							<XIcon aria-hidden="true" className="size-4" />
						</Button>
					</Hint>
				</section>
			</Panel>
		</>
	);
});
