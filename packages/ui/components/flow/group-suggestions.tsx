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
import XIcon from "lucide-react/dist/esm/icons/x.js";
import {
	memo,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
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
import { typeToColor } from "./utils";

type Point = { x: number; y: number };

export interface GroupSuggestionsOverlayProps {
	suggestions: GroupSuggestion[];
	selectedId?: string;
	preview: boolean;
	busy: boolean;
	onSelect: (id: string) => void;
	onPreview: () => void;
	onCollapse: () => void;
	onDismiss: () => void;
	onClose: () => void;
	getPinPosition?: (nodeId: string, pinId: string) => Point | undefined;
}

const GROUP_PADDING = 20;

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

export const GroupSuggestionsOverlay = memo(function GroupSuggestionsOverlay({
	suggestions,
	selectedId,
	preview,
	busy,
	onSelect,
	onPreview,
	onCollapse,
	onDismiss,
	onClose,
	getPinPosition,
}: GroupSuggestionsOverlayProps) {
	const { t } = useTranslation("flow");
	const stripRef = useRef<HTMLElement>(null);
	const nodes = useNodes();
	const { getInternalNode } = useReactFlow();
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
	const selected =
		visible.find((suggestion) => suggestion.id === selectedId) ?? visible[0];

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

	useEffect(() => {
		const strip = stripRef.current;
		const board = strip?.closest(".react-flow");
		const document = strip?.ownerDocument;
		if (!strip || !document) return;
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key !== "Escape" || event.defaultPrevented || event.isComposing)
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
			event.preventDefault();
			event.stopPropagation();
			onClose();
		};
		document.addEventListener("keydown", onKeyDown, true);
		return () => document.removeEventListener("keydown", onKeyDown, true);
	}, [onClose]);

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
					className="nodrag nopan nowheel pointer-events-auto flex flex-wrap items-center gap-2 rounded-xl border border-border bg-background/95 px-3 py-2 shadow-lg backdrop-blur-sm"
					onPointerDown={(event) => event.stopPropagation()}
				>
					<div className="mr-2 min-w-0 flex-1" aria-live="polite">
						<p className="truncate text-sm font-medium">
							{selected?.label ?? t("groupSuggestions", "Group suggestions")}
						</p>
						<p className="text-xs text-muted-foreground">
							{selected
								? t(
										"groupSuggestionCounts",
										"{{nodes}} nodes · {{wires}} internal wires · {{inputs}} in / {{outputs}} out",
										{
											nodes: selected.nodeCount,
											wires: selected.internalEdgeCount,
											inputs: selected.inputCount,
											outputs: selected.outputCount,
										},
									)
								: t(
										"noGroupSuggestions",
										"No groups to suggest in this layer.",
									)}
						</p>
					</div>
					{selected && (
						<>
							<Button
								type="button"
								size="sm"
								variant="outline"
								disabled={busy}
								aria-pressed={preview}
								onClick={onPreview}
							>
								{preview
									? t("hideGroupPreview", "Hide preview")
									: t("previewGroup", "Preview")}
							</Button>
							<Button
								type="button"
								size="sm"
								disabled={busy}
								onClick={onCollapse}
							>
								{busy
									? t("collapsingGroup", "Collapsing…")
									: t("collapseGroup", "Collapse")}
							</Button>
							<Button
								type="button"
								size="sm"
								variant="ghost"
								disabled={busy}
								onClick={onDismiss}
							>
								{t("dismissGroup", "Dismiss")}
							</Button>
						</>
					)}
					<Button
						type="button"
						size="icon"
						variant="ghost"
						className="size-8"
						aria-label={t("closeGroupSuggestions", "Close group suggestions")}
						onClick={onClose}
					>
						<XIcon aria-hidden="true" className="size-4" />
					</Button>
				</section>
			</Panel>
		</>
	);
});
