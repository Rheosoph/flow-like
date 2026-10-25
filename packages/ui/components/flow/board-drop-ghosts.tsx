"use client";

import {
	type DragMoveEvent,
	type UniqueIdentifier,
	useDndMonitor,
} from "@dnd-kit/core";
import { Position, getBezierPath } from "@xyflow/react";
import { WorkflowIcon } from "lucide-react";
import { memo, useMemo, useState } from "react";
import {
	type Palette,
	type SceneNode,
	sceneFromFlow,
} from "../../lib/board-canvas/scene";
import {
	type BoardDropPayload,
	boardDropPayload,
	planBoardDrop,
	withFunctionPins,
} from "../../lib/board-drop";
import type { ILayer, IVariable } from "../../lib/schema/flow/board";
import type { INode } from "../../lib/schema/flow/node";
import type { IPin } from "../../lib/schema/flow/pin";
import { storageDisplayPath } from "../../lib/storage-tree";
import { DynamicImage } from "../ui";
import { useReactFlow, useStore } from "./board-runtime";
import { typeToColor } from "./utils";

interface ShellDrag {
	id: UniqueIdentifier;
	payload: BoardDropPayload;
	/** Client coordinates of the pointer, as the drop handler receives them. */
	point: { x: number; y: number };
}

interface GhostWire {
	key: string;
	path: string;
	color: string;
}

/** The nodes a drop creates, laid out around the drop point at the origin. */
interface GhostPlan {
	nodes: SceneNode[];
	pins: ReadonlyMap<string, IPin>;
	wires: GhostWire[];
	caption?: string;
}

/** The scene only supplies geometry here; the ghost colors pins from the theme. */
const GEOMETRY_PALETTE: Palette = {
	background: [0, 0, 0, 0],
	card: [0, 0, 0, 0],
	foreground: [0, 0, 0, 0],
	muted: [0, 0, 0, 0],
	border: [0, 0, 0, 0],
	primary: [0, 0, 0, 0],
	tertiary: [0, 0, 0, 0],
};

function planGhosts(
	payload: BoardDropPayload,
	catalog: readonly INode[] | undefined,
	layers: Record<string, ILayer> | undefined,
): GhostPlan | undefined {
	const nodes = planBoardDrop(payload, catalog)?.map((node) =>
		payload.type === "function-layer"
			? withFunctionPins(node, layers?.[payload.layerId])
			: node,
	);
	if (!nodes?.length) return undefined;
	const scene = sceneFromFlow(
		nodes.map((node) => ({
			id: node.id,
			type: "node",
			position: {
				x: node.coordinates?.[0] ?? 0,
				y: node.coordinates?.[1] ?? 0,
			},
			data: { node },
		})),
		[],
		GEOMETRY_PALETTE,
	);
	const pins = new Map(
		nodes.flatMap((node) =>
			Object.values(node.pins).map((pin) => [pin.id, pin] as const),
		),
	);
	const anchors = new Map(
		scene.nodes.flatMap((node) =>
			node.pins.map(
				(pin) => [pin.id, { x: node.x + pin.x, y: node.y + pin.y }] as const,
			),
		),
	);
	const wires = [...pins.values()].flatMap((pin) =>
		pin.connected_to.flatMap((target) => {
			const from = anchors.get(pin.id);
			const to = anchors.get(target);
			if (!from || !to) return [];
			const [path] = getBezierPath({
				sourceX: from.x,
				sourceY: from.y,
				sourcePosition: Position.Right,
				targetX: to.x,
				targetY: to.y,
				targetPosition: Position.Left,
			});
			return [
				{ key: `${pin.id}-${target}`, path, color: typeToColor(pin.data_type) },
			];
		}),
	);
	const caption =
		payload.type === "variable"
			? payload.variable.name
			: payload.type === "storage-path"
				? storageDisplayPath(payload.path)
				: undefined;
	return { nodes: scene.nodes, pins, wires, caption };
}

/**
 * Translucent previews of the nodes a shell drag creates, at the size and place the drop
 * gives them. They are planned like the drop itself and drawn above both renderers.
 */
export const BoardDropGhosts = memo(function BoardDropGhosts({
	catalog,
	layers,
	pending,
}: {
	catalog: readonly INode[] | undefined;
	layers: Record<string, ILayer> | undefined;
	/** A dropped variable keeps its preview while the user picks Get or Set. */
	pending?: { variable: IVariable; screenPosition: { x: number; y: number } };
}) {
	const flow = useReactFlow();
	const [tx, ty, zoom] = useStore((state) => state.transform);
	const [drag, setDrag] = useState<ShellDrag>();
	const monitor = useMemo(
		() => ({
			onDragMove({ active, over, activatorEvent, delta }: DragMoveEvent) {
				const pointer = activatorEvent as PointerEvent;
				setDrag((current) => {
					if (over?.id !== "flow") return undefined;
					const payload =
						current?.id === active.id
							? current.payload
							: boardDropPayload(active.data.current);
					return (
						payload && {
							id: active.id,
							payload,
							point: {
								x: pointer.clientX + delta.x,
								y: pointer.clientY + delta.y,
							},
						}
					);
				});
			},
			onDragEnd: () => setDrag(undefined),
			onDragCancel: () => setDrag(undefined),
		}),
		[],
	);
	useDndMonitor(monitor);
	const pendingPayload = useMemo<BoardDropPayload | undefined>(
		() => pending && { type: "variable", variable: pending.variable },
		[pending],
	);
	const payload = drag?.payload ?? pendingPayload;
	const point = drag?.point ?? pending?.screenPosition;
	const plan = useMemo(
		() => payload && planGhosts(payload, catalog, layers),
		[payload, catalog, layers],
	);

	if (!plan || !point) return null;
	const origin = flow.screenToFlowPosition(point);
	return (
		<div
			className="pointer-events-none absolute inset-0 z-30 overflow-hidden"
			data-board-drop-ghosts=""
			aria-hidden="true"
		>
			<div
				className="absolute left-0 top-0 origin-top-left"
				style={{ transform: `translate(${tx}px, ${ty}px) scale(${zoom})` }}
			>
				<div
					className="absolute left-0 top-0"
					style={{ transform: `translate(${origin.x}px, ${origin.y}px)` }}
				>
					<GhostScene plan={plan} />
				</div>
			</div>
		</div>
	);
});

const GhostScene = memo(function GhostScene({ plan }: { plan: GhostPlan }) {
	const first = plan.nodes[0];
	return (
		<>
			<svg
				width="1"
				height="1"
				className="absolute overflow-visible"
				role="presentation"
			>
				{plan.wires.map((wire) => (
					<path
						key={wire.key}
						d={wire.path}
						fill="none"
						stroke={wire.color}
						strokeWidth={1.5}
						strokeDasharray="5 4"
						opacity={0.7}
					/>
				))}
			</svg>
			{plan.nodes.map((node) => (
				<GhostCard key={node.id} node={node} pins={plan.pins} />
			))}
			{plan.caption && first && (
				<span
					className="absolute max-w-60 truncate text-[0.6rem] leading-3 text-muted-foreground"
					style={{ left: first.x, top: first.y + first.height + 4 }}
				>
					{plan.caption}
				</span>
			)}
		</>
	);
});

const GhostCard = memo(function GhostCard({
	node,
	pins,
}: {
	node: SceneNode;
	pins: ReadonlyMap<string, IPin>;
}) {
	return (
		<div
			className="absolute rounded-md border border-dashed border-muted-foreground/60 bg-card/50 opacity-70 shadow-sm"
			data-drop-ghost=""
			style={{
				left: node.x,
				top: node.y,
				width: node.width,
				height: node.height,
			}}
		>
			<div className="flex h-4 min-w-0 items-center gap-1 border-b border-dashed border-muted-foreground/40 px-1">
				{node.icon ? (
					<DynamicImage
						className="size-2 shrink-0 bg-foreground"
						url={node.icon}
					/>
				) : (
					<WorkflowIcon className="size-2 shrink-0" />
				)}
				<span className="truncate text-[0.6rem] font-medium leading-none">
					{node.label}
				</span>
			</div>
			{node.pins.map((pin) => {
				const source = pins.get(pin.id);
				if (!source || pin.kind === "reference") return null;
				const input = pin.direction === "input";
				return (
					<div key={pin.id}>
						{pin.kind === "execution" ? (
							<span
								className="absolute size-2 -translate-x-1/2 -translate-y-1/2 rotate-45 rounded-[1.5px] bg-foreground"
								style={{ left: pin.x, top: pin.y }}
							/>
						) : (
							<span
								className="absolute size-1.75 -translate-x-1/2 -translate-y-1/2 rounded-full"
								style={{
									left: pin.x,
									top: pin.y,
									backgroundColor: typeToColor(source.data_type),
								}}
							/>
						)}
						{pin.label && (
							<span
								className={`absolute truncate text-[0.6rem] leading-3 -translate-y-1/2 ${input ? "text-start" : "-translate-x-full text-end"}`}
								style={{
									left: input ? pin.x + 8 : pin.x - 6.8,
									top: pin.y,
									maxWidth: pin.labelMaxWidth,
								}}
							>
								{pin.label}
							</span>
						)}
					</div>
				);
			})}
		</div>
	);
});
