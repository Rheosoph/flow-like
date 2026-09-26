"use client";
import {
	type InternalNode,
	type NodeChange,
	type NodePositionChange,
	type ReactFlowState,
	type XYPosition,
	useStore,
	useStoreApi,
} from "@xyflow/react";
import {
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useSyncExternalStore,
} from "react";
import {
	type GuideBox,
	type HelperLine,
	type PinGuide,
	pickReferences,
	snapToGuides,
	unionBox,
} from "../../lib/flow-helper-lines";
import {
	type SignalStore,
	createSignalStore,
} from "../../lib/realtime/presence-signals-store";
import { cn } from "../../lib/utils";

const SNAP_DISTANCE_PX = 6;
const NEAREST_REFERENCES = 12;
const LINE_OVERSHOOT_PX = 8;
const NO_LINES: readonly HelperLine[] = [];

interface DragModifiers {
	shiftKey: boolean;
	metaKey: boolean;
	ctrlKey: boolean;
}

interface WiredPin {
	nodeId: string;
	offset: XYPosition;
	target: XYPosition;
}

interface HelperLineSession {
	starts: Map<string, XYPosition>;
	sizes: Map<string, { width: number; height: number }>;
	references: GuideBox[];
	wired: Set<GuideBox>;
	pins: WiredPin[];
	lockAxis: boolean;
	bypass: boolean;
}

type DraggingChange = NodePositionChange & { position: XYPosition };

const isDragging = (change: NodeChange): change is DraggingChange =>
	change.type === "position" && Boolean(change.dragging && change.position);

function boxOf(node: InternalNode): GuideBox | undefined {
	const { width, height } = node.measured;
	if (!width || !height) return undefined;
	return { ...node.internals.positionAbsolute, width, height };
}

function handleCentre(
	node: InternalNode | undefined,
	handleId?: string | null,
): XYPosition | undefined {
	const bounds = node?.internals.handleBounds;
	if (!handleId || !bounds) return undefined;
	const handle = [...(bounds.source ?? []), ...(bounds.target ?? [])].find(
		(candidate) => candidate.id === handleId,
	);
	if (!handle) return undefined;
	return { x: handle.x + handle.width / 2, y: handle.y + handle.height / 2 };
}

/**
 * Everything a drag can align to, read once at drag start: the drag moves
 * nothing else, so per-frame work is a pass over plain boxes. Nodes carried
 * along with the drag (a comment's contents) are neither references nor wires.
 */
function createSession(
	{ nodeLookup, edges }: ReactFlowState,
	moving: ReadonlySet<string>,
	carried: ReadonlySet<string>,
): HelperLineSession {
	const starts = new Map<string, XYPosition>();
	const sizes = new Map<string, { width: number; height: number }>();
	const boxes = new Map<string, GuideBox>();
	for (const [id, node] of nodeLookup) {
		if (moving.has(id)) {
			starts.set(id, { ...node.position });
			sizes.set(id, {
				width: node.measured.width ?? 0,
				height: node.measured.height ?? 0,
			});
			continue;
		}
		if (node.hidden || carried.has(id)) continue;
		const box = boxOf(node);
		if (box) boxes.set(id, box);
	}

	const wired = new Set<GuideBox>();
	const pins: WiredPin[] = [];
	for (const edge of edges) {
		if (edge.hidden) continue;
		const sourceMoves = moving.has(edge.source);
		if (sourceMoves === moving.has(edge.target)) continue;
		const [nodeId, handleId, fixedId, fixedHandleId] = sourceMoves
			? [edge.source, edge.sourceHandle, edge.target, edge.targetHandle]
			: [edge.target, edge.targetHandle, edge.source, edge.sourceHandle];
		const fixed = boxes.get(fixedId);
		if (!fixed) continue;
		wired.add(fixed);
		const offset = handleCentre(nodeLookup.get(nodeId), handleId);
		const target = handleCentre(nodeLookup.get(fixedId), fixedHandleId);
		if (!offset || !target) continue;
		pins.push({
			nodeId,
			offset,
			target: { x: fixed.x + target.x, y: fixed.y + target.y },
		});
	}

	return {
		starts,
		sizes,
		references: [...boxes.values()],
		wired,
		pins,
		lockAxis: false,
		bypass: false,
	};
}

function readModifiers(session: HelperLineSession, event: DragModifiers) {
	session.lockAxis = event.shiftKey;
	session.bypass = event.metaKey || event.ctrlKey;
}

/** ⇧ keeps the drag on the axis it has moved furthest along. */
function lockToAxis(
	session: HelperLineSession,
	dragging: readonly DraggingChange[],
): { x: boolean; y: boolean } {
	const lead = dragging[0];
	const origin = session.starts.get(lead.id);
	if (!session.lockAxis || !origin) return { x: true, y: true };
	const horizontal =
		Math.abs(lead.position.x - origin.x) > Math.abs(lead.position.y - origin.y);
	for (const change of dragging) {
		const start = session.starts.get(change.id);
		if (!start) continue;
		if (horizontal) change.position.y = start.y;
		else change.position.x = start.x;
	}
	return { x: horizontal, y: !horizontal };
}

export interface HelperLines {
	lines: SignalStore<readonly HelperLine[]>;
	begin: (
		event: DragModifiers,
		dragged: readonly { id: string }[],
		carried?: Iterable<string>,
	) => void;
	/** Call with every node change batch, before it is applied. */
	snap: (changes: readonly NodeChange[]) => void;
	end: () => void;
}

/**
 * Alignment guides for node drags. Snapping holds ⌘/Ctrl off, ⇧ locks the
 * axis. Lines go through a signal store so drag frames never re-render the
 * board for them.
 */
export function useHelperLines(): HelperLines {
	const store = useStoreApi();
	const lines = useMemo(
		() => createSignalStore<readonly HelperLine[]>(NO_LINES),
		[],
	);
	const sessionRef = useRef<HelperLineSession | undefined>(undefined);
	const detachRef = useRef<(() => void) | undefined>(undefined);

	const end = useCallback(() => {
		sessionRef.current = undefined;
		detachRef.current?.();
		detachRef.current = undefined;
		lines.set(NO_LINES);
	}, [lines]);

	const begin = useCallback<HelperLines["begin"]>(
		(event, dragged, carried = []) => {
			end();
			const session = createSession(
				store.getState(),
				new Set(dragged.map((node) => node.id)),
				new Set(carried),
			);
			readModifiers(session, event);
			sessionRef.current = session;
			const track = (next: Event) => {
				readModifiers(session, next as KeyboardEvent | PointerEvent);
				if (session.bypass) lines.set(NO_LINES);
			};
			// pointerup also covers drags xyflow aborts without an onNodeDragStop.
			const listeners: [string, EventListener][] = [
				["pointermove", track],
				["keydown", track],
				["keyup", track],
				["pointerup", end],
				["pointercancel", end],
			];
			const options = { capture: true, passive: true };
			for (const [type, listener] of listeners)
				window.addEventListener(type, listener, options);
			detachRef.current = () => {
				for (const [type, listener] of listeners)
					window.removeEventListener(type, listener, options);
			};
		},
		[store, lines, end],
	);

	const snap = useCallback<HelperLines["snap"]>(
		(changes) => {
			const session = sessionRef.current;
			if (!session) return;
			const dragging = changes.filter(
				(change): change is DraggingChange =>
					isDragging(change) && session.starts.has(change.id),
			);
			if (dragging.length === 0) return;

			const axes = lockToAxis(session, dragging);
			if (session.bypass) {
				lines.set(NO_LINES);
				return;
			}

			const moving = unionBox(
				dragging.map((change) => ({
					...change.position,
					width: session.sizes.get(change.id)?.width ?? 0,
					height: session.sizes.get(change.id)?.height ?? 0,
				})),
			);
			if (!moving) return;
			const positions = new Map(
				dragging.map((change) => [change.id, change.position]),
			);
			const pins: PinGuide[] = [];
			for (const pin of session.pins) {
				const position = positions.get(pin.nodeId);
				if (!position) continue;
				pins.push({
					x: position.x + pin.offset.x,
					y: position.y + pin.offset.y,
					targetX: pin.target.x,
					targetY: pin.target.y,
				});
			}

			const {
				transform: [tx, ty, zoom],
				width,
				height,
			} = store.getState();
			const references = pickReferences(session.references, moving, {
				visible: {
					x: -tx / zoom,
					y: -ty / zoom,
					width: width / zoom,
					height: height / zoom,
				},
				limit: NEAREST_REFERENCES,
				always: session.wired,
			});
			const {
				dx,
				dy,
				lines: next,
			} = snapToGuides(moving, references, {
				threshold: SNAP_DISTANCE_PX / zoom,
				axes,
				pins,
			});
			// In place: xyflow hands out its drag item's own position object, and
			// that object is what onNodeDragStop reports and the board commits.
			for (const change of dragging) {
				change.position.x += dx;
				change.position.y += dy;
			}
			lines.set(next.length > 0 ? next : NO_LINES);
		},
		[store, lines],
	);

	useEffect(() => end, [end]);

	return useMemo(
		() => ({ lines, begin, snap, end }),
		[lines, begin, snap, end],
	);
}

export function FlowHelperLinesLayer({
	lines,
}: Readonly<{ lines: SignalStore<readonly HelperLine[]> }>) {
	const current = useSyncExternalStore(
		lines.subscribe,
		lines.getSnapshot,
		lines.getSnapshot,
	);
	if (current.length === 0) return null;
	return <HelperLinesSvg lines={current} />;
}

const HelperLinesSvg = memo(function HelperLinesSvg({
	lines,
}: Readonly<{ lines: readonly HelperLine[] }>) {
	const [tx, ty, zoom] = useStore((state) => state.transform);
	return (
		<svg
			aria-hidden="true"
			className="pointer-events-none absolute inset-0 z-30 size-full overflow-visible"
		>
			{lines.map((line) => {
				const at = line.at * zoom + (line.axis === "x" ? tx : ty);
				const shift = line.axis === "x" ? ty : tx;
				const overshoot = line.pin ? 0 : LINE_OVERSHOOT_PX;
				const from = line.from * zoom + shift - overshoot;
				const to = line.to * zoom + shift + overshoot;
				const [x1, y1, x2, y2] =
					line.axis === "x" ? [at, from, at, to] : [from, at, to, at];
				return (
					<line
						key={`${line.axis}:${line.at}:${line.from}:${line.to}`}
						x1={x1}
						y1={y1}
						x2={x2}
						y2={y2}
						strokeWidth={1}
						shapeRendering="crispEdges"
						className={cn(
							"stroke-primary",
							line.pin && "[stroke-dasharray:4_3]",
						)}
					/>
				);
			})}
		</svg>
	);
});
