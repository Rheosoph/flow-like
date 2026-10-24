"use client";

import {
	type Node,
	type NodeChange,
	Position,
	type Viewport,
} from "@xyflow/react";
import {
	type MouseEvent as ReactMouseEvent,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useTranslation } from "react-i18next";
import { betweenFrames } from "../../lib/board-canvas/frames";
import {
	type IconRaster,
	NODE_GLYPHS,
	rasterizeGlyph,
	rasterizeIcon,
} from "../../lib/board-canvas/icon-raster";
import { iconRasterSize } from "../../lib/board-canvas/icons";
import {
	BoardHitIndex,
	ConnectionGesture,
	type Point,
	boundsInPolygon,
} from "../../lib/board-canvas/interaction";
import {
	type BoardCanvasHandle,
	loadBoardCanvas,
	loadBoardCanvasFonts,
} from "../../lib/board-canvas/load";
import {
	type PinStart,
	resolvePinStart,
} from "../../lib/board-canvas/pin-start";
import { pinchViewport } from "../../lib/board-canvas/pinch";
import type { Overlay, Palette, Scene } from "../../lib/board-canvas/scene";
import { waitForCanvasPaint } from "../../lib/board-canvas/startup";
import {
	type NodeStatus,
	collectRunActivity,
} from "../../lib/board-canvas/status";
import { useBoardRuntime, useReactFlow, useStoreApi } from "./board-runtime";
import type { FlowCanvasProps } from "./flow-board";

export interface EguiCanvasLayerProps {
	scene: Scene;
	palette: Palette;
	boardProps: FlowCanvasProps;
	lod?: boolean;
	active?: boolean;
	paintLayer?: "all" | "edges" | "nodes";
	interactive?: boolean;
	/** Run state, logs and lint marks per node id, painted without rebuilding the scene. */
	status?: Readonly<Record<string, NodeStatus>>;
	/** Live runs, for the "last activity" clock on running nodes. */
	runs?: ReadonlyMap<string, { lastNodeUpdateMs: number }>;
	onHoverNode?(id?: string): void;
	onError(error: Error): void;
	onReady(): void;
}

const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

interface PointerDrag {
	id: number;
	start: Point;
	last: Point;
	startWorld: Point;
	mode: "connection" | "node" | "pan" | "selection";
	viewport: Viewport;
	nodes: Node[];
	started: boolean;
	secondClick?: boolean;
	appliedWorld?: Point;
	appliedModifiers?: string;
	positions?: Map<string, Point>;
	lasso?: Point[];
	selectionMode?: "box" | "lasso";
	cameraMoved?: boolean;
	toggleNode?: string;
	primaryId?: string;
	pane?: boolean;
}

const asMouse = (event: MouseEvent | KeyboardEvent) =>
	event as unknown as ReactMouseEvent;
const updateError = (part: string, error: unknown) =>
	new Error(
		`The canvas could not update ${part}: ${error instanceof Error ? error.message : String(error)}`,
		{ cause: error },
	);
/** Draws the graph and forwards editing intents to the existing board services. */
export function EguiCanvasLayer(props: EguiCanvasLayerProps) {
	const { t } = useTranslation("flow");
	const translate = useRef(t);
	translate.current = t;
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const inputRef = useRef<HTMLDivElement>(null);
	const lassoRef = useRef<SVGPolygonElement>(null);
	const latest = useRef(props);
	latest.current = props;
	const originalEdges = useMemo(
		() =>
			new Map(
				props.interactive === false
					? []
					: props.boardProps.edges?.map((edge) => [edge.id, edge]),
			),
		[props.boardProps.edges, props.interactive],
	);
	const originalEdgesRef = useRef(originalEdges);
	originalEdgesRef.current = originalEdges;
	const handleRef = useRef<BoardCanvasHandle | undefined>(undefined);
	const [initialIndex] = useState(
		() => new BoardHitIndex({ nodes: [], edges: [] }),
	);
	const hitRef = useRef(initialIndex);
	const cancelRef = useRef<(restore: boolean) => void>(() => {});
	const repaintRef = useRef<() => void>(() => {});
	const flow = useReactFlow();
	const runtime = useBoardRuntime();
	const store = useStoreApi();
	const [announcement, setAnnouncement] = useState("");
	const [pending, setPending] = useState(false);
	const sentIcons = useRef(new Set<string>());
	/** Graph nodes the canvas indexes; comments and media move in the DOM only. */
	const sceneIds = useRef(new Set<string>());
	const statusFrame = useRef(0);
	const sendStatus = useRef(() => {});
	sendStatus.current = () => {
		statusFrame.current = 0;
		const handle = handleRef.current;
		if (!handle?.set_status) return;
		try {
			handle.set_status(
				JSON.stringify({
					nodes: latest.current.status ?? {},
					motion: !window.matchMedia(REDUCED_MOTION).matches,
				}),
			);
		} catch (error) {
			// Missing run marks must not cost the board its native renderer.
			console.error(updateError("the run state", error));
		}
	};
	const sendActivity = useRef(() => {});
	sendActivity.current = () => {
		const handle = handleRef.current;
		if (!handle?.set_activity) return;
		try {
			handle.set_activity(
				JSON.stringify(
					collectRunActivity(latest.current.runs ?? new Map(), Date.now()),
				),
			);
		} catch (error) {
			console.error(updateError("the run activity", error));
		}
	};
	const syncIcons = useRef(() => {});
	syncIcons.current = () => {
		const handle = handleRef.current;
		if (!handle?.set_icon || props.paintLayer === "edges") return;
		const size = iconRasterSize(
			store.getState().maxZoom,
			globalThis.devicePixelRatio ?? 1,
		);
		const send = (
			key: string,
			raster: (size: number) => Promise<IconRaster>,
		) => {
			const sent = `${size}:${key}`;
			if (sentIcons.current.has(sent)) return;
			sentIcons.current.add(sent);
			raster(size)
				.then((icon) => {
					if (handleRef.current !== handle) return;
					handle.set_icon?.(key, icon.width, icon.height, icon.rgba, icon.tint);
				})
				.catch((error: unknown) => {
					sentIcons.current.delete(sent);
					// The painter keeps its vector fallback; the board stays usable.
					console.warn(`Canvas icon ${key} is unavailable`, error);
				});
		};
		for (const key of Object.keys(NODE_GLYPHS))
			send(key, (size) => rasterizeGlyph(key, size));
		for (const node of props.scene.nodes) {
			const url = node.icon;
			if (url && !node.dom) send(url, (size) => rasterizeIcon(url, size));
		}
	};

	useEffect(() => {
		if (props.interactive !== false) {
			const nativeIds = new Set(props.scene.nodes.map((node) => node.id));
			sceneIds.current = nativeIds;
			const obstacles = (props.boardProps.nodes ?? []).flatMap((node) => {
				if (node.hidden || nativeIds.has(node.id)) return [];
				const internal = flow.getInternalNode(node.id);
				const position = internal?.internals.positionAbsolute ?? node.position;
				return [
					{
						id: node.id,
						...position,
						width:
							internal?.measured.width ??
							node.measured?.width ??
							node.width ??
							0,
						height:
							internal?.measured.height ??
							node.measured?.height ??
							node.height ??
							0,
					},
				];
			});
			hitRef.current = new BoardHitIndex(props.scene, obstacles);
		}
		try {
			handleRef.current?.set_scene(JSON.stringify(props.scene));
			syncIcons.current();
			repaintRef.current();
		} catch (error) {
			props.onError(updateError("the scene", error));
		}
	}, [
		props.scene,
		props.boardProps.nodes,
		props.onError,
		props.interactive,
		flow,
	]);
	// A run updates its stores many times a frame; the canvas takes the latest once per frame.
	// biome-ignore lint/correctness/useExhaustiveDependencies: The sender reads the latest status.
	useEffect(() => {
		const flush = () => sendStatus.current();
		if (!statusFrame.current)
			statusFrame.current = requestAnimationFrame(flush);
	}, [props.status]);
	useEffect(() => {
		const motion = window.matchMedia(REDUCED_MOTION);
		const resend = () => sendStatus.current();
		motion.addEventListener("change", resend);
		return () => {
			motion.removeEventListener("change", resend);
			cancelAnimationFrame(statusFrame.current);
			statusFrame.current = 0;
		};
	}, []);
	useEffect(() => {
		const tick = () => sendActivity.current();
		tick();
		if (!props.runs?.size) return;
		const timer = setInterval(tick, 1000);
		return () => clearInterval(timer);
	}, [props.runs]);
	useEffect(() => {
		try {
			handleRef.current?.set_palette(JSON.stringify(props.palette));
			handleRef.current?.set_lod(props.lod ?? true);
		} catch (error) {
			props.onError(error instanceof Error ? error : new Error(String(error)));
		}
	}, [props.palette, props.lod, props.onError]);
	useEffect(() => {
		if (!props.active || props.paintLayer !== "edges") return;
		const handle = handleRef.current;
		if (!handle?.pulse) return;
		const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
		let timer: ReturnType<typeof setTimeout> | undefined;
		const stop = () => {
			clearTimeout(timer);
			timer = undefined;
			handle.stop_pulse?.();
		};
		const sweep = () => {
			if (document.hidden || motion.matches) return;
			handle.pulse?.();
			// A 1.2 second sweep is followed by nine seconds without animation requests.
			timer = setTimeout(sweep, 10_200);
		};
		const resume = () => {
			stop();
			sweep();
		};
		document.addEventListener("visibilitychange", resume);
		motion.addEventListener("change", resume);
		sweep();
		return () => {
			document.removeEventListener("visibilitychange", resume);
			motion.removeEventListener("change", resume);
			stop();
		};
	}, [props.active, props.paintLayer]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: A scope or read-only change invalidates the active gesture.
	useEffect(() => {
		cancelRef.current(true);
	}, [props.boardProps.scopeKey, props.boardProps.nodesInteractive]);

	useEffect(() => {
		const canvas = canvasRef.current;
		const root = canvas?.closest<HTMLElement>(".react-flow");
		if (!canvas || !root) return;
		const interactive = props.interactive !== false;
		const applePlatform = /Mac|iPod|iPhone|iPad/.test(navigator.platform);
		const pane = root.querySelector<HTMLElement>(".react-flow__pane") ?? root;
		const pinHint = document.createElement("div");
		pinHint.dataset.boardPinHover = "";
		Object.assign(pinHint.style, {
			position: "absolute",
			inset: "0",
			pointerEvents: "none",
			zIndex: "100000",
			display: "none",
		});
		const pinRing = document.createElement("div");
		Object.assign(pinRing.style, {
			position: "absolute",
			width: "16px",
			height: "16px",
			border: "2px solid var(--primary)",
			borderRadius: "50%",
			boxShadow: "0 0 0 2px var(--background)",
			transform: "translate(-50%, -50%)",
		});
		const pinLabel = document.createElement("div");
		pinLabel.dataset.boardPinLabel = "";
		Object.assign(pinLabel.style, {
			position: "absolute",
			maxWidth: "240px",
			padding: "3px 7px",
			border: "1px solid var(--border)",
			borderRadius: "5px",
			background: "var(--popover)",
			color: "var(--popover-foreground)",
			font: "11px/16px var(--font-sans, sans-serif)",
			whiteSpace: "nowrap",
			overflow: "hidden",
			textOverflow: "ellipsis",
		});
		pinHint.append(pinRing, pinLabel);
		if (interactive)
			(runtime?.store.getState().overlayElement ?? root).append(pinHint);
		let pinCursor:
			| { element: HTMLElement | SVGElement; previous: string }
			| undefined;
		const clearPinFeedback = () => {
			const visible = pinHint.style.display !== "none";
			if (pinCursor) pinCursor.element.style.cursor = pinCursor.previous;
			pinCursor = undefined;
			if (visible && pane.style.cursor === "crosshair") pane.style.cursor = "";
			pinHint.style.display = "none";
			delete pinHint.dataset.nodeId;
			delete pinHint.dataset.pinId;
			return visible;
		};
		const showPinFeedback = (candidate: PinStart, target: Element | null) => {
			const { pin } = candidate;
			const element =
				target instanceof HTMLElement || target instanceof SVGElement
					? target
					: pane;
			if (pinCursor?.element !== element) {
				clearPinFeedback();
				pinCursor = { element, previous: element.style.cursor };
			}
			element.style.cursor = "crosshair";
			const [x, y, zoom] = store.getState().transform;
			const px = x + pin.x * zoom;
			const py = y + pin.y * zoom;
			pinRing.style.left = `${px}px`;
			pinRing.style.top = `${py}px`;
			pinLabel.style.left = `${Math.max(4, Math.min(px + 14, store.getState().width - 244))}px`;
			pinLabel.style.top = `${Math.max(4, Math.min(py + 12, store.getState().height - 30))}px`;
			const direction =
				pin.direction === "input"
					? translate.current("pinHintInput", "input")
					: translate.current("pinHintOutput", "output");
			const name =
				pin.label ||
				(pin.kind === "reference"
					? translate.current("pinHintReference", "Function reference")
					: pin.kind === "execution"
						? translate.current("pinHintExecution", "Execution")
						: pin.id);
			pinLabel.textContent = candidate.reconnect
				? translate.current(
						"pinHintReconnect",
						"Reconnect {{direction}} · {{name}}",
						{ direction, name },
					)
				: translate.current("pinHintConnect", "Drag {{direction}} · {{name}}", {
						direction,
						name,
					});
			pinHint.dataset.nodeId = pin.nodeId;
			pinHint.dataset.pinId = pin.id;
			pinHint.style.display = "block";
		};
		let disposed = false;
		let cancelStartupWait: (() => void) | undefined;
		let ready = false;
		let busy = false;
		let pointer: Point | undefined;
		let lastPointerEvent: PointerEvent | undefined;
		let drag: PointerDrag | undefined;
		const touches = new Map<number, Point>();
		let pinch:
			| {
					ids: [number, number];
					distance: number;
					zoom: number;
					anchor: Point;
					ended: boolean;
			  }
			| undefined;
		let connection: ConnectionGesture | undefined;
		let waitingForClick = false;
		let space = false;
		let suppressClick = false;
		let frame: (() => void) | undefined;
		let cameraHover: ReturnType<typeof setTimeout> | undefined;
		const clearCameraHover = () => {
			clearTimeout(cameraHover);
			cameraHover = undefined;
		};
		let longPress: ReturnType<typeof setTimeout> | undefined;
		const clearLongPress = () => {
			clearTimeout(longPress);
			longPress = undefined;
		};
		let lastFrameTime = 0;
		let previousTransform = store.getState().transform;
		let wasInteractive =
			store.getState().nodesDraggable || store.getState().nodesConnectable;
		let scope = latest.current.boardProps.scopeKey;
		const currentProps = () => latest.current.boardProps;
		let hoveredNode: string | undefined;
		let hoverPaused = false;
		const hoverNode = (id?: string) => {
			if (hoveredNode === id && !hoverPaused) return;
			hoverPaused = false;
			hoveredNode = id;
			latest.current.onHoverNode?.(id);
		};
		const clearHover = (event: PointerEvent) => {
			if (event.target !== root || drag || pinch || event.buttons) return;
			clearPinFeedback();
			setOverlay({});
			hoverNode();
			if (!connection) {
				// A later measurement must not restore hover at an old in-board position.
				pointer = undefined;
				lastPointerEvent = undefined;
			}
		};
		const world = (client: Point): Point => {
			const bounds = root.getBoundingClientRect();
			const [x, y, zoom] = store.getState().transform;
			return {
				x: (client.x - bounds.left - x) / zoom,
				y: (client.y - bounds.top - y) / zoom,
			};
		};
		const nativeAboveElement = (point: Point, element: Element | null) => {
			if (!latest.current.onHoverNode) return undefined;
			const node = hitRef.current.nodeAt(point);
			if (!node || node.dom) return undefined;
			const wrapper = element?.closest<HTMLElement>(".react-flow__node");
			if (!wrapper || wrapper.dataset.id === node.id) return undefined;
			const nativeZ = flow.getInternalNode(node.id)?.internals.z ?? 20;
			const domZ =
				flow.getInternalNode(wrapper.dataset.id ?? "")?.internals.z ??
				Number(getComputedStyle(wrapper).zIndex);
			return nativeZ > domZ ? node : undefined;
		};
		const validate = (
			candidate: Parameters<
				NonNullable<FlowCanvasProps["isValidConnection"]>
			>[0],
		) =>
			!!currentProps().nodesInteractive &&
			store.getState().nodesConnectable &&
			[candidate.source, candidate.target].every((id) => {
				const node = store.getState().nodeLookup.get(id);
				return node && !node.hidden && node.connectable !== false;
			}) &&
			[
				{ nodeId: candidate.source, pinId: candidate.sourceHandle },
				{ nodeId: candidate.target, pinId: candidate.targetHandle },
			].every((endpoint) => {
				if (!endpoint.pinId) return false;
				const pin = hitRef.current.pin({
					nodeId: endpoint.nodeId,
					pinId: endpoint.pinId,
				});
				if (!pin) return false;
				const bounds = root.getBoundingClientRect();
				const [x, y, zoom] = store.getState().transform;
				const top = document.elementFromPoint(
					bounds.left + x + pin.x * zoom,
					bounds.top + y + pin.y * zoom,
				);
				if (!top || !root.contains(top)) return false;
				if (nativeAboveElement(pin, top)?.id === endpoint.nodeId) return true;
				if (isDomControl(top)) return false;
				const coveredBy =
					top.closest<HTMLElement>(".react-flow__node")?.dataset.id;
				return !coveredBy || coveredBy === endpoint.nodeId;
			}) &&
			(currentProps().isValidConnection?.(candidate) ?? false);
		const report = (error: unknown) => {
			if (!disposed)
				latest.current.onError(
					error instanceof Error ? error : new Error(String(error)),
				);
		};
		const viewportChanged = () =>
			currentProps().onMoveEnd?.(null, flow.getViewport());
		const moveViewport = (viewport: Viewport) => {
			if (drag && !drag.cameraMoved) {
				drag.cameraMoved = true;
				store.getState().onMoveStart?.(null, flow.getViewport());
			}
			return runtime
				? runtime.setViewport(viewport, { notify: false })
				: flow.setViewport(viewport);
		};
		const paintPinch = () => {
			if (!pinch || pinch.ended) return;
			const a = touches.get(pinch.ids[0]);
			const b = touches.get(pinch.ids[1]);
			if (!a || !b) return;
			const bounds = root.getBoundingClientRect();
			const state = store.getState();
			const viewport = pinchViewport(
				pinch.zoom,
				pinch.distance,
				pinch.anchor,
				{
					x: (a.x + b.x) / 2 - bounds.left,
					y: (a.y + b.y) / 2 - bounds.top,
				},
				Math.hypot(a.x - b.x, a.y - b.y),
				state.minZoom,
				state.maxZoom,
			);
			void moveViewport(viewport);
			state.onMove?.(null, viewport);
		};
		const setOverlay = (overlay: Overlay) => {
			try {
				handleRef.current?.set_overlay(JSON.stringify(overlay));
			} catch (error) {
				report(updateError("the interaction overlay", error));
			}
		};

		const paintOverlay = () => {
			if (!ready || !pointer || pinch) return;
			if (scope !== currentProps().scopeKey) {
				cancel(false);
				scope = currentProps().scopeKey;
				return;
			}
			if (
				drag?.mode === "node" &&
				drag.nodes.some((node) => !store.getState().nodeLookup.has(node.id))
			) {
				cancel(true);
				setAnnouncement("Drag cancelled because a selected item was removed.");
				return;
			}
			const point = world(pointer);
			const zoom = flow.getViewport().zoom;
			if (connection) {
				clearPinFeedback();
				if (
					!currentProps().nodesInteractive ||
					!store.getState().nodesConnectable ||
					store.getState().nodeLookup.get(connection.anchor.nodeId)
						?.connectable === false
				) {
					cancel(true);
					return;
				}
				const resolved = connection.resolve(
					hitRef.current,
					point,
					zoom,
					validate,
				);
				if (!resolved) {
					cancel(false);
					setAnnouncement("Connection cancelled because its endpoint changed.");
					return;
				}
				const { anchor, target, invalid } = resolved;
				const highlighted = target ?? invalid;
				const direction = anchor.direction === "output" ? -1 : 1;
				const end = target ?? {
					x: point.x + (direction * 9) / zoom,
					y: point.y - 7 / zoom,
				};
				setOverlay({
					connection: {
						from: [anchor.x, anchor.y],
						to: [end.x, end.y],
						fromDirection: anchor.direction,
						valid: target ? true : invalid ? false : undefined,
						color: anchor.color,
						kind: anchor.kind,
					},
					hoveredPin: highlighted
						? {
								nodeId: highlighted.nodeId,
								pinId: highlighted.id,
								valid: !!target,
							}
						: undefined,
				});
				pane.style.cursor = invalid
					? "not-allowed"
					: target
						? "crosshair"
						: "grabbing";
			} else if (drag?.mode === "selection") {
				const view = flow.getViewport();
				lassoRef.current?.setAttribute(
					"points",
					(drag.lasso ?? [])
						.map(
							(p) => `${p.x * view.zoom + view.x},${p.y * view.zoom + view.y}`,
						)
						.join(" "),
				);
			} else if (!drag && !lastPointerEvent?.buttons) {
				const top = document.elementFromPoint(pointer.x, pointer.y);
				const covered = nativeAboveElement(point, top);
				const wrapperId =
					top?.closest<HTMLElement>(".react-flow__node")?.dataset.id;
				const hit =
					covered ??
					(!isDomControl(top) &&
					(!wrapperId || hitRef.current.nodes.has(wrapperId))
						? hitRef.current.nodeAt(point)
						: undefined);
				hoverNode(hit?.id);
				const candidate = pinStart(point, zoom, top);
				if (candidate) showPinFeedback(candidate, top);
				else clearPinFeedback();
				pane.style.cursor = candidate
					? "crosshair"
					: hitRef.current.nodeAt(point)
						? "pointer"
						: "grab";
				setOverlay(
					candidate
						? {
								hoveredPin: {
									nodeId: candidate.pin.nodeId,
									pinId: candidate.pin.id,
									valid: true,
								},
							}
						: {},
				);
			}
		};
		repaintRef.current = paintOverlay;

		function cancel(restore: boolean, preserveTouches = false) {
			clearCameraHover();
			clearPinFeedback();
			clearLongPress();
			hoverNode();
			if (
				restore &&
				drag?.mode === "node" &&
				drag.started &&
				scope === currentProps().scopeKey
			) {
				currentProps().onNodesChange?.(
					drag.nodes.map((node) => ({
						id: node.id,
						type: "position",
						position: node.position,
						dragging: false,
					})),
				);
			}
			if (drag?.mode === "node" && drag.started)
				currentProps().onNativeCancelDrag?.(
					restore && scope === currentProps().scopeKey,
				);
			space = false;
			lastPointerEvent = undefined;
			if (drag?.cameraMoved) viewportChanged();
			const captured = drag?.id;
			drag = undefined;
			connection = undefined;
			waitingForClick = false;
			lassoRef.current?.setAttribute("points", "");
			if (
				!preserveTouches &&
				captured !== undefined &&
				root?.hasPointerCapture(captured)
			)
				root.releasePointerCapture(captured);
			if (!preserveTouches) {
				if (pinch && !pinch.ended) viewportChanged();
				pinch = undefined;
				const ids = [...touches.keys()];
				touches.clear();
				for (const id of ids)
					if (root?.hasPointerCapture(id)) root.releasePointerCapture(id);
			}
			pane.style.cursor = "";
			setOverlay({});
		}
		cancelRef.current = cancel;

		const selected = (
			ids: Set<string>,
			additive: boolean,
			edgeId?: string,
			incidentEdges = false,
		) => {
			if (!store.getState().elementsSelectable) return;
			const bp = currentProps();
			const nodes = bp.nodes ?? [];
			const edges = bp.edges ?? [];
			const changes: NodeChange[] = [];
			for (const node of nodes) {
				const next =
					!node.hidden &&
					node.selectable !== false &&
					(ids.has(node.id) || (additive && !!node.selected));
				if (!!node.selected !== next)
					changes.push({ id: node.id, type: "select", selected: next });
			}
			if (changes.length) bp.onNodesChange?.(changes);
			const edgeSelected = (edge: (typeof edges)[number]) =>
				!edge.hidden &&
				edge.selectable !== false &&
				(edge.id === edgeId
					? !(additive && edge.selected)
					: (incidentEdges && (ids.has(edge.source) || ids.has(edge.target))) ||
						(additive && !!edge.selected));
			bp.onEdgesChange?.(
				edges
					.filter((edge) => !!edge.selected !== edgeSelected(edge))
					.map((edge) => ({
						id: edge.id,
						type: "select",
						selected: edgeSelected(edge),
					})),
			);
			if (!runtime)
				bp.onSelectionChange?.({
					nodes: nodes.filter(
						(node) =>
							!node.hidden &&
							node.selectable !== false &&
							(ids.has(node.id) || (additive && node.selected)),
					),
					edges: edges.filter((edge) => edgeSelected(edge)),
				});
		};

		const moveDrag = (event: PointerEvent) => {
			if (!drag) return;
			pointer = { x: event.clientX, y: event.clientY };
			drag.last = pointer;
			const dx = pointer.x - drag.start.x;
			const dy = pointer.y - drag.start.y;
			if (drag.mode === "pan") {
				if (Math.hypot(dx, dy) > 2) drag.started = true;
				void moveViewport({
					...drag.viewport,
					x: drag.viewport.x + dx,
					y: drag.viewport.y + dy,
				});
			} else if (drag.mode === "node") {
				if (!drag.started && Math.hypot(dx, dy) < 3) return;
				if (
					!currentProps().nodesInteractive ||
					!store.getState().nodesDraggable
				) {
					cancel(true);
					return;
				}
				if (!drag.started) {
					drag.started = true;
					currentProps().onNodeDragStart?.(
						asMouse(event),
						drag.nodes.find((node) => node.id === drag?.primaryId) ??
							drag.nodes[0],
						drag.nodes,
					);
				}
				const point = world(pointer);
				const modifiers = `${event.shiftKey}:${event.metaKey}:${event.ctrlKey}`;
				if (
					drag.appliedWorld?.x === point.x &&
					drag.appliedWorld.y === point.y &&
					drag.appliedModifiers === modifiers
				)
					return;
				drag.appliedWorld = point;
				drag.appliedModifiers = modifiers;
				const startWorld = drag.startWorld;
				const changes: NodeChange[] = drag.nodes.map((node) => ({
					id: node.id,
					type: "position",
					position: {
						x: node.position.x + point.x - startWorld.x,
						y: node.position.y + point.y - startWorld.y,
					},
					dragging: true,
				}));
				// The existing helper-line callback adjusts these positions before applying them.
				currentProps().onNodesChange?.(changes);
				const positions = changes.flatMap((change) =>
					change.type === "position" && change.position
						? [{ id: change.id, ...change.position }]
						: [],
				);
				drag.positions = new Map(
					positions.map((position) => [
						position.id,
						{ x: position.x, y: position.y },
					]),
				);
				hitRef.current.moveNodes(positions);
				try {
					handleRef.current?.set_positions(
						JSON.stringify(
							positions.filter((position) => sceneIds.current.has(position.id)),
						),
					);
				} catch (error) {
					report(updateError("node positions", error));
					return;
				}
				const moved = drag.nodes.map((node) => ({
					...node,
					position: positions.find((p) => p.id === node.id) ?? node.position,
				}));
				currentProps().onNodeDrag?.(
					asMouse(event),
					moved.find((node) => node.id === drag?.primaryId) ?? moved[0],
					moved,
				);
			} else if (drag.mode === "selection") {
				const point = world(pointer);
				if (drag.selectionMode === "box") {
					drag.lasso = [
						drag.startWorld,
						{ x: point.x, y: drag.startWorld.y },
						point,
						{ x: drag.startWorld.x, y: point.y },
					];
					paintOverlay();
					return;
				}
				const previous = drag.lasso?.at(-1);
				if (
					!previous ||
					Math.hypot(point.x - previous.x, point.y - previous.y) *
						flow.getViewport().zoom >=
						2
				)
					drag.lasso?.push(point);
			}
			paintOverlay();
		};

		const animate = (time: number) => {
			frame = undefined;
			const dt = Math.min(32, time - (lastFrameTime || time));
			lastFrameTime = time;
			if (pinch) paintPinch();
			else if (lastPointerEvent && drag && drag.mode !== "pan")
				moveDrag(lastPointerEvent);
			if (
				drag &&
				(drag.mode === "connection" || drag.mode === "node") &&
				pointer
			) {
				const bounds = root.getBoundingClientRect();
				const speed = (coordinate: number, min: number, max: number) =>
					coordinate < min + 36
						? Math.min(1, (min + 36 - coordinate) / 36)
						: coordinate > max - 36
							? -Math.min(1, (coordinate - max + 36) / 36)
							: 0;
				const vx = speed(pointer.x, bounds.left, bounds.right);
				const vy = speed(pointer.y, bounds.top, bounds.bottom);
				if (vx || vy) {
					const view = flow.getViewport();
					void moveViewport({
						...view,
						x: view.x + vx * dt * 0.6,
						y: view.y + vy * dt * 0.6,
					});
				}
				frame = betweenFrames(animate);
			}
			paintOverlay();
		};
		// Between frames: changed inside a frame callback, the canvases would draw a frame late.
		const schedule = () => {
			frame ??= betweenFrames(animate);
		};

		const isDomControl = (target: EventTarget | null) =>
			target instanceof Element &&
			!!target.closest(
				".react-flow__panel, .react-flow__node-toolbar, [data-board-canvas-controls], [data-board-dom-control], button, input, textarea, select, a, [contenteditable='true'], [role='dialog'], [role='button'], [role='slider'], [role='combobox'], [role='menuitem']",
			);
		const pinStart = (point: Point, zoom: number, target: Element | null) => {
			const state = store.getState();
			if (busy || !currentProps().nodesInteractive || !state.nodesConnectable)
				return;
			const above = nativeAboveElement(point, target);
			const wrapperId =
				target?.closest<HTMLElement>(".react-flow__node")?.dataset.id;
			const handle = !above
				? target?.closest<HTMLElement>(".react-flow__handle")
				: undefined;
			return resolvePinStart(hitRef.current, point, zoom, {
				blocked:
					!above &&
					(isDomControl(target) ||
						(!!wrapperId && !hitRef.current.nodes.has(wrapperId)) ||
						(!!handle && !handle.classList.contains("connectablestart"))),
				explicit:
					handle?.dataset.nodeid && handle.dataset.handleid
						? { nodeId: handle.dataset.nodeid, pinId: handle.dataset.handleid }
						: undefined,
				explicitEdgeId:
					!above &&
					target?.closest(
						".react-flow__edge-path, .react-flow__edge-interaction",
					)
						? target.closest<SVGElement>(".react-flow__edge")?.dataset.id
						: undefined,
				nodeId: above?.id ?? wrapperId ?? hitRef.current.nodeAt(point)?.id,
				canConnect: (pin) => {
					const node = state.nodeLookup.get(pin.nodeId);
					return !!node && !node.hidden && node.connectable !== false;
				},
				canReconnect: (edge, end) => {
					const original = originalEdgesRef.current.get(edge.id);
					return (
						!!original &&
						!original.hidden &&
						(original.reconnectable === undefined ||
							original.reconnectable === true ||
							original.reconnectable === end)
					);
				},
			});
		};
		const isControl = (target: EventTarget | null) =>
			isDomControl(target) ||
			(target instanceof Element && !!target.closest(".nodrag"));
		const nativeSurface = (event: MouseEvent) => {
			const element =
				event.target instanceof Element ? event.target : undefined;
			const underlyingNode = element?.closest<HTMLElement>(".react-flow__node");
			if (underlyingNode) {
				const z =
					flow.getInternalNode(underlyingNode.dataset.id ?? "")?.internals.z ??
					Number(getComputedStyle(underlyingNode).zIndex);
				if (z < 18) {
					const edge = hitRef.current.edgeAt(
						world({ x: event.clientX, y: event.clientY }),
						flow.getViewport().zoom,
					);
					if (edge && !edge.dom) return true;
				}
			}
			if (
				nativeAboveElement(
					world({ x: event.clientX, y: event.clientY }),
					element ?? null,
				)
			)
				return true;
			// Production pin editors are nested inside Handle, so their control boundary wins.
			if (isDomControl(event.target)) return false;
			// Pins remain draggable connection targets even when their editors use nodrag.
			if (!element?.closest(".react-flow__handle") && isControl(event.target))
				return false;
			if (element?.closest(".react-flow__nodesselection")) return false;
			const wrapper = element?.closest<HTMLElement>(".react-flow__node");
			if (!wrapper) return true;
			return store.getState().nodeLookup.has(wrapper.dataset.id ?? "");
		};
		const consume = (event: Event) => {
			event.preventDefault();
			event.stopImmediatePropagation();
		};
		const targetNode = (event: MouseEvent, point: Point) => {
			const above = nativeAboveElement(
				point,
				event.target instanceof Element ? event.target : null,
			);
			if (above) return above;
			const id =
				event.target instanceof Element
					? event.target.closest<HTMLElement>(".react-flow__node")?.dataset.id
					: undefined;
			if (
				id &&
				(flow.getInternalNode(id)?.internals.z ?? 20) < 18 &&
				hitRef.current.edgeAt(point, flow.getViewport().zoom)
			)
				return undefined;
			return (
				(id && (hitRef.current.nodes.get(id) ?? flow.getNode(id))) ||
				hitRef.current.nodeAt(point)
			);
		};
		const down = (event: PointerEvent) => {
			if (
				ready &&
				!busy &&
				event.pointerType === "touch" &&
				!isDomControl(event.target) &&
				(pinch || nativeSurface(event))
			) {
				if (pinch && !pinch.ids.includes(event.pointerId)) {
					consume(event);
					return;
				}
				touches.set(event.pointerId, { x: event.clientX, y: event.clientY });
				if (!pinch && touches.size === 2) {
					const [[firstId, a], [secondId, b]] = [...touches];
					cancel(true, true);
					const center = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
					pinch = {
						ids: [firstId, secondId],
						distance: Math.hypot(a.x - b.x, a.y - b.y),
						zoom: flow.getViewport().zoom,
						anchor: world(center),
						ended: false,
					};
					pointer = undefined;
					lastPointerEvent = undefined;
					hoverPaused = true;
					suppressClick = true;
					inputRef.current?.focus({ preventScroll: true });
					for (const id of pinch.ids) root.setPointerCapture(id);
					store.getState().onMoveStart?.(event, flow.getViewport());
				}
				if (pinch) {
					consume(event);
					return;
				}
			}
			// Control-click opens the existing context menu on Apple platforms.
			if (
				!ready ||
				event.button === 2 ||
				(applePlatform && event.button === 0 && event.ctrlKey)
			)
				return;
			const selectionGroup =
				event.target instanceof Element &&
				!!event.target.closest(".react-flow__nodesselection-rect") &&
				!isControl(event.target);
			const selection =
				event.button === 0 &&
				event.shiftKey &&
				!connection &&
				!space &&
				store.getState().elementsSelectable &&
				!isControl(event.target) &&
				!(event.target instanceof Element && event.target.closest(".nokey"));
			if (
				!selection &&
				!selectionGroup &&
				!nativeSurface(event) &&
				!(connection && waitingForClick && !isControl(event.target))
			)
				return;
			if (busy) {
				consume(event);
				return;
			}
			clearPinFeedback();
			setOverlay({});
			if (drag && drag.id !== event.pointerId) {
				cancel(true);
				return;
			}
			pointer = { x: event.clientX, y: event.clientY };
			const point = world(pointer);
			const zoom = flow.getViewport().zoom;
			const bp = currentProps();
			const additive = event.metaKey || event.ctrlKey;
			const base: PointerDrag = {
				id: event.pointerId,
				start: pointer,
				last: pointer,
				startWorld: point,
				viewport: flow.getViewport(),
				mode: "pan",
				nodes: [],
				started: false,
				pane: true,
			};
			if (selection) {
				base.mode = "selection";
				base.selectionMode = bp.selectionMode ?? "box";
				base.lasso = [point];
			} else if (connection && waitingForClick) {
				base.mode = "connection";
				base.secondClick = true;
			} else if (
				selectionGroup &&
				event.button === 0 &&
				!space &&
				bp.nodesInteractive &&
				store.getState().nodesDraggable
			) {
				base.nodes = (bp.nodes ?? [])
					.filter(
						(node) => node.selected && !node.hidden && node.draggable !== false,
					)
					.map((node) => ({ ...node, position: { ...node.position } }));
				if (base.nodes.length) {
					base.mode = "node";
					base.primaryId = base.nodes[0].id;
					base.pane = false;
				}
			} else if (event.button === 0 && !space) {
				const underPointer = targetNode(event, point);
				const candidate = pinStart(
					point,
					zoom,
					event.target instanceof Element ? event.target : null,
				);
				const endpoint = candidate?.reconnect;
				const pin = candidate?.pin;
				if (endpoint) {
					connection = new ConnectionGesture(
						endpoint.edge[endpoint.end === "source" ? "target" : "source"],
						{ edgeId: endpoint.edge.id, end: endpoint.end },
					);
					base.mode = "connection";
				} else if (pin) {
					connection = new ConnectionGesture({
						nodeId: pin.nodeId,
						pinId: pin.id,
					});
					base.mode = "connection";
				} else {
					const node = underPointer;
					const edgeId =
						event.target instanceof Element
							? event.target.closest<HTMLElement>(".react-flow__edge")?.dataset
									.id
							: undefined;
					const edge = node
						? undefined
						: ((edgeId ? hitRef.current.edges.get(edgeId) : undefined) ??
							hitRef.current.edgeAt(point, zoom));
					base.pane = !node && !edge;
					if (node) {
						base.primaryId = node.id;
						const current = bp.nodes?.find((n) => n.id === node.id);
						if (current) {
							if (
								additive &&
								current.selected &&
								store.getState().elementsSelectable
							) {
								base.toggleNode = node.id;
							}
							if (!current.selected) selected(new Set([node.id]), additive);
							if (
								bp.nodesInteractive &&
								store.getState().nodesDraggable &&
								current.draggable !== false &&
								(!current.dragHandle ||
									(event.target instanceof Element &&
										!!event.target.closest(current.dragHandle)))
							) {
								base.mode = "node";
								base.nodes =
									(current.selected || additive
										? bp.nodes?.filter(
												(n) =>
													(n.selected || n.id === current.id) &&
													!n.hidden &&
													n.draggable !== false,
											)
										: [current]
									)?.map((n) => ({ ...n, position: { ...n.position } })) ?? [];
							}
						}
					} else if (edge) selected(new Set(), additive, edge.id);
					else if (!event.altKey && !additive) selected(new Set(), false);
				}
			}
			drag = base;
			hoverPaused = true;
			lastPointerEvent = event;
			waitingForClick = false;
			suppressClick = true;
			const focusNode =
				base.mode === "node"
					? root.querySelector<HTMLElement>(
							`.react-flow__node[data-id="${CSS.escape(base.nodes[0]?.id ?? "")}"]`,
						)
					: undefined;
			(focusNode ?? inputRef.current)?.focus({ preventScroll: true });
			root.setPointerCapture(event.pointerId);
			if (event.pointerType !== "mouse" && base.mode !== "connection") {
				clearLongPress();
				longPress = setTimeout(() => {
					if (drag !== base || base.started) return;
					cancel(true);
					root.dispatchEvent(
						new MouseEvent("contextmenu", {
							bubbles: true,
							cancelable: true,
							view: window,
							clientX: event.clientX,
							clientY: event.clientY,
							button: 2,
						}),
					);
				}, 700);
			}
			consume(event);
			paintOverlay();
			schedule();
		};
		const move = (event: PointerEvent) => {
			if (event.pointerType === "touch" && touches.has(event.pointerId)) {
				touches.set(event.pointerId, { x: event.clientX, y: event.clientY });
				if (pinch) {
					consume(event);
					if (!pinch.ended) schedule();
					return;
				}
			}
			if (!ready || (drag && drag.id !== event.pointerId)) return;
			if (
				drag &&
				Math.hypot(
					event.clientX - drag.start.x,
					event.clientY - drag.start.y,
				) >= 4
			)
				clearLongPress();
			pointer = { x: event.clientX, y: event.clientY };
			// Keep the freehand path at input cadence; a frame can span several corners.
			if (drag?.mode === "selection" && drag.selectionMode === "lasso") {
				const coalesced = event.getCoalescedEvents?.();
				for (const sample of coalesced?.length ? coalesced : [event]) {
					const point = world({ x: sample.clientX, y: sample.clientY });
					const previous = drag.lasso?.at(-1);
					if (
						!previous ||
						Math.hypot(point.x - previous.x, point.y - previous.y) *
							flow.getViewport().zoom >=
							2
					)
						drag.lasso?.push(point);
				}
			}
			lastPointerEvent = event;
			if (drag) consume(event);
			// Moved before egui's frame callback, the camera is painted in the same frame as the DOM.
			if (drag?.mode === "pan") moveDrag(event);
			schedule();
		};
		const commitConnection = async (event: PointerEvent) => {
			if (!connection || !pointer) return;
			if (
				!currentProps().nodesInteractive ||
				!store.getState().nodesConnectable
			) {
				cancel(true);
				return;
			}
			const action = connection.drop(
				hitRef.current,
				world(pointer),
				flow.getViewport().zoom,
				validate,
			);
			connection = undefined;
			waitingForClick = false;
			setOverlay({});
			const bp = currentProps();
			const commitScope = bp.scopeKey;
			try {
				if (action.kind === "connect" || action.kind === "disconnect") {
					const old =
						action.kind === "disconnect"
							? bp.edges?.find((edge) => edge.id === action.edgeId)
							: action.oldEdgeId
								? bp.edges?.find((edge) => edge.id === action.oldEdgeId)
								: undefined;
					if ((action.kind === "disconnect" || action.oldEdgeId) && !old)
						return;
					busy = true;
					setPending(true);
					if (action.kind === "connect")
						await bp.onNativeConnection(action.connection, old);
					else if (old) await bp.onNativeDisconnect(old);
					if (!disposed && commitScope === currentProps().scopeKey)
						setAnnouncement(
							action.kind === "connect"
								? "Connection saved."
								: "Connection removed.",
						);
				} else if (action.kind === "palette") {
					const pin = action.pin;
					const fromNode = flow.getInternalNode(pin.nodeId);
					if (!fromNode) return;
					const fromPosition =
						pin.kind === "reference"
							? pin.direction === "output"
								? Position.Bottom
								: Position.Top
							: pin.direction === "output"
								? Position.Right
								: Position.Left;
					const toPosition =
						pin.kind === "reference"
							? pin.direction === "output"
								? Position.Top
								: Position.Bottom
							: pin.direction === "output"
								? Position.Left
								: Position.Right;
					bp.onConnectEnd?.(event, {
						isValid: false,
						from: { x: pin.x, y: pin.y },
						fromHandle: {
							id: pin.id,
							nodeId: pin.nodeId,
							type: pin.direction === "output" ? "source" : "target",
							position: fromPosition,
							x: pin.x,
							y: pin.y,
							width: 1,
							height: 1,
						},
						fromPosition,
						fromNode,
						to: world(pointer),
						toHandle: null,
						toPosition,
						toNode: null,
						pointer: world(pointer),
					});
				} else
					setAnnouncement(
						action.kind === "noop"
							? "Connection unchanged."
							: "Connection cancelled.",
					);
			} catch (error) {
				if (!disposed)
					setAnnouncement(
						error instanceof Error
							? error.message
							: "The connection could not be saved.",
					);
			} finally {
				// Let the confirmed board revision reach React before accepting another commit.
				requestAnimationFrame(() => {
					busy = false;
					if (!disposed) setPending(false);
				});
			}
		};
		const up = (event: PointerEvent) => {
			if (event.pointerType === "touch" && touches.has(event.pointerId)) {
				if (pinch) {
					consume(event);
					paintPinch();
					if (!pinch.ended) {
						pinch.ended = true;
						viewportChanged();
					}
					touches.delete(event.pointerId);
					if (root.hasPointerCapture(event.pointerId))
						root.releasePointerCapture(event.pointerId);
					if (!touches.size) {
						pinch = undefined;
						pointer = undefined;
						lastPointerEvent = undefined;
						pane.style.cursor = "";
					}
					return;
				}
				touches.delete(event.pointerId);
			}
			clearLongPress();
			if (!drag || drag.id !== event.pointerId) return;
			consume(event);
			moveDrag(event);
			if (!drag) return;
			const completed = drag;
			drag = undefined;
			lastPointerEvent = undefined;
			if (root.hasPointerCapture(event.pointerId))
				root.releasePointerCapture(event.pointerId);
			if (completed.toggleNode && !completed.started)
				currentProps().onNodesChange?.([
					{ id: completed.toggleNode, type: "select", selected: false },
				]);
			if (completed.mode === "connection") {
				if (
					!connection?.reconnect &&
					!completed.secondClick &&
					Math.hypot(
						event.clientX - completed.start.x,
						event.clientY - completed.start.y,
					) < 4
				) {
					waitingForClick = true;
					setAnnouncement("Choose a compatible pin. Escape cancels.");
				} else void commitConnection(event);
			} else if (completed.mode === "node" && completed.started) {
				const moved = completed.nodes.map((node) => {
					return {
						...node,
						position: completed.positions?.get(node.id) ?? node.position,
					};
				});
				currentProps().onNodesChange?.(
					moved.map((node) => ({
						id: node.id,
						type: "position",
						position: node.position,
						dragging: false,
					})),
				);
				currentProps().onNodeDragStop?.(
					asMouse(event),
					moved.find((node) => node.id === completed.primaryId) ?? moved[0],
					moved,
				);
			} else if (completed.mode === "selection" && pointer) {
				selected(
					new Set(
						(currentProps().nodes ?? [])
							.filter((node) => {
								if (node.hidden || node.selectable === false) return false;
								const internal = flow.getInternalNode(node.id);
								const position =
									internal?.internals.positionAbsolute ?? node.position;
								return boundsInPolygon(
									{
										...position,
										width:
											internal?.measured.width ??
											node.measured?.width ??
											node.width ??
											0,
										height:
											internal?.measured.height ??
											node.measured?.height ??
											node.height ??
											0,
									},
									completed.lasso ?? [],
								);
							})
							.map((node) => node.id),
					),
					event.metaKey || event.ctrlKey,
					undefined,
					true,
				);
				lassoRef.current?.setAttribute("points", "");
				store.setState({ nodesSelectionActive: true });
			} else if (
				completed.mode === "pan" &&
				!completed.started &&
				completed.pane
			)
				currentProps().onPaneClick?.(asMouse(event));
			viewportChanged();
			paintOverlay();
		};
		const cancelled = () => {
			cancel(true);
			setAnnouncement("Gesture cancelled.");
		};
		const click = (event: MouseEvent) => {
			if (suppressClick) {
				suppressClick = false;
				if (isDomControl(event.target)) return;
				if (
					!isControl(event.target) ||
					(event.target instanceof Element &&
						event.target.closest(".react-flow__handle"))
				)
					consume(event);
			}
		};
		const doubleClick = (event: MouseEvent) => {
			if (!ready || !nativeSurface(event)) return;
			const point = world({ x: event.clientX, y: event.clientY });
			const node = targetNode(event, point);
			const edge = node
				? undefined
				: hitRef.current.edgeAt(point, flow.getViewport().zoom);
			const bp = currentProps();
			if (node) {
				const flowNode = bp.nodes?.find((n) => n.id === node.id);
				if (flowNode) bp.onNodeDoubleClick?.(asMouse(event), flowNode);
			} else if (edge) {
				const flowEdge = bp.edges?.find((e) => e.id === edge.id);
				if (flowEdge) bp.onEdgeDoubleClick?.(asMouse(event), flowEdge);
			}
			if (node || edge) consume(event);
		};
		const contextMenu = (event: MouseEvent) => {
			if (connection) {
				cancel(true);
				consume(event);
				return;
			}
			const svgEdge =
				event.target instanceof Element
					? event.target.closest<HTMLElement>(".react-flow__edge")?.dataset.id
					: undefined;
			// Selection can replace the event's SVG/DOM target. Let the menu receive
			// this event before changing which renderer owns that target.
			if (ready && svgEdge && !isDomControl(event.target)) {
				const edge = hitRef.current.edges.get(svgEdge);
				if (edge && !edge.selected)
					setTimeout(() => {
						if (!disposed) selected(new Set(), false, edge.id);
					}, 0);
				return;
			}
			if (!ready || !nativeSurface(event)) return;
			const node = targetNode(
				event,
				world({ x: event.clientX, y: event.clientY }),
			);
			if (node && !node.selected)
				setTimeout(() => {
					if (!disposed) selected(new Set([node.id]), false);
				}, 0);
			if (!node) {
				const edge = hitRef.current.edgeAt(
					world({ x: event.clientX, y: event.clientY }),
					flow.getViewport().zoom,
				);
				if (edge && !edge.selected)
					setTimeout(() => {
						if (!disposed) selected(new Set(), false, edge.id);
					}, 0);
			}
		};
		const keyDown = (event: KeyboardEvent) => {
			if (!root.contains(document.activeElement) || isControl(event.target))
				return;
			if ((connection || drag || pinch) && event.key.startsWith("Arrow")) {
				consume(event);
				return;
			}
			if (event.key === "Escape" && (drag || connection || pinch)) {
				cancelled();
				consume(event);
			}
			if (event.code === "Space") {
				space = true;
				event.preventDefault();
			}
		};
		const keyUp = (event: KeyboardEvent) => {
			if (event.code === "Space") space = false;
		};
		const wheel = () => {
			if (connection) schedule();
		};
		const onLostCapture = (event: PointerEvent) => {
			if (
				drag?.id === event.pointerId ||
				(pinch && touches.has(event.pointerId))
			)
				cancelled();
		};
		const onVisibility = () => {
			if (document.hidden) cancelled();
		};
		const unsubscribe = store.subscribe((state) => {
			if (
				(connection && !state.nodesConnectable) ||
				(drag?.mode === "node" && !state.nodesDraggable)
			)
				cancel(true);
			const editable = state.nodesDraggable || state.nodesConnectable;
			if (wasInteractive && !editable) {
				wasInteractive = editable;
				cancel(true);
			} else wasInteractive = editable;
			if (state.transform !== previousTransform) {
				previousTransform = state.transform;
				try {
					handleRef.current?.set_viewport(...state.transform);
					clearCameraHover();
					if (interactive) {
						if (connection || drag || pinch) paintOverlay();
						else if (pointer) {
							if (clearPinFeedback()) setOverlay({});
							// Idle hover reads DOM hit targets. Reading after each camera write
							// forces layout; refresh once the stationary-pointer zoom settles.
							cameraHover = setTimeout(() => {
								cameraHover = undefined;
								if (!disposed) paintOverlay();
							}, 160);
						}
					}
				} catch (error) {
					report(error);
				}
			}
		});
		if (interactive) {
			root.addEventListener("pointerdown", down, true);
			root.addEventListener("pointermove", move, true);
			root.addEventListener("pointerleave", clearHover, true);
			root.addEventListener("pointerup", up, true);
			root.addEventListener("pointercancel", cancelled, true);
			root.addEventListener("lostpointercapture", onLostCapture, true);
			root.addEventListener("click", click, true);
			root.addEventListener("dblclick", doubleClick, true);
			root.addEventListener("contextmenu", contextMenu, true);
			root.addEventListener("wheel", wheel, { passive: true });
			window.addEventListener("keydown", keyDown, true);
			window.addEventListener("keyup", keyUp, true);
			window.addEventListener("blur", cancelled);
			document.addEventListener("visibilitychange", onVisibility);
		}
		const lostContext = (event: Event) => {
			event.preventDefault();
			cancel(true);
			report(
				new Error(
					"The GPU canvas context was lost. React Flow has been restored.",
				),
			);
		};
		canvas.addEventListener("webglcontextlost", lostContext);
		void Promise.all([
			loadBoardCanvas(),
			loadBoardCanvasFonts(),
			document.fonts.ready,
		])
			.then(async ([module, fonts]) => {
				if (disposed) return;
				const startingScene = latest.current.scene;
				const startingPalette = latest.current.palette;
				const handle = await module.start(
					canvas,
					JSON.stringify(startingScene),
					JSON.stringify(startingPalette),
				);
				if (disposed) {
					handle.destroy();
					return;
				}
				handleRef.current = handle;
				sentIcons.current.clear();
				handle.set_layer(latest.current.paintLayer ?? "all");
				// Prop effects can run while WebRunner starts and no handle exists yet.
				if (latest.current.scene !== startingScene)
					handle.set_scene(JSON.stringify(latest.current.scene));
				if (latest.current.palette !== startingPalette)
					handle.set_palette(JSON.stringify(latest.current.palette));
				syncIcons.current();
				sendStatus.current();
				sendActivity.current();
				const renderedFrames = () => {
					const value = handle.stats();
					const stats = typeof value === "string" ? JSON.parse(value) : value;
					if (
						!stats ||
						typeof stats.frames !== "number" ||
						!Number.isFinite(stats.frames)
					)
						throw new Error("The native canvas did not report a frame count.");
					return stats.frames;
				};
				const initialFrames = renderedFrames();
				handle.set_fonts(...fonts);
				handle.set_lod(latest.current.lod ?? true);
				handle.set_viewport(...store.getState().transform);
				// WebRunner does not paint hidden tabs. Spend the deadline only while
				// the configured canvas can draw, and request a fresh frame on return.
				const startup = waitForCanvasPaint({
					canvas,
					renderedFrames,
					initialFrames,
					onDrawable: () => handle.set_viewport(...store.getState().transform),
				});
				cancelStartupWait = startup.cancel;
				await startup.promise;
				cancelStartupWait = undefined;
				if (disposed) return;
				ready = true;
				latest.current.onReady();
			})
			.catch(report);

		return () => {
			disposed = true;
			cancelStartupWait?.();
			ready = false;
			cancel(true);
			pinHint.remove();
			frame?.();
			unsubscribe();
			root.removeEventListener("pointerdown", down, true);
			root.removeEventListener("pointermove", move, true);
			root.removeEventListener("pointerleave", clearHover, true);
			root.removeEventListener("pointerup", up, true);
			root.removeEventListener("pointercancel", cancelled, true);
			root.removeEventListener("lostpointercapture", onLostCapture, true);
			root.removeEventListener("click", click, true);
			root.removeEventListener("dblclick", doubleClick, true);
			root.removeEventListener("contextmenu", contextMenu, true);
			root.removeEventListener("wheel", wheel);
			window.removeEventListener("keydown", keyDown, true);
			window.removeEventListener("keyup", keyUp, true);
			window.removeEventListener("blur", cancelled);
			document.removeEventListener("visibilitychange", onVisibility);
			canvas.removeEventListener("webglcontextlost", lostContext);
			handleRef.current?.destroy();
			handleRef.current = undefined;
			repaintRef.current = () => {};
			cancelRef.current = () => {};
		};
	}, [flow, runtime, store, props.interactive]);

	return (
		<div
			ref={inputRef}
			tabIndex={props.interactive === false ? undefined : 0}
			data-board-canvas-input={props.interactive === false ? undefined : "true"}
			role={props.interactive === false ? undefined : "application"}
			aria-label={
				props.interactive === false
					? undefined
					: "Workflow canvas. Drag from a pin to connect. Select a wire to reconnect either endpoint. Shift-drag selects. Escape cancels."
			}
			style={{
				position: "absolute",
				inset: 0,
				pointerEvents: "none",
				outline: "none",
			}}
		>
			<canvas
				ref={canvasRef}
				tabIndex={-1}
				aria-hidden="true"
				data-testid={
					props.interactive === false ? "egui-board-edges" : "egui-board-canvas"
				}
				style={{
					position: "absolute",
					inset: 0,
					width: "100%",
					height: "100%",
					zIndex: 0,
					pointerEvents: "none",
					outline: "none",
				}}
			/>
			{props.interactive !== false && (
				<svg
					aria-hidden="true"
					className="absolute inset-0 size-full pointer-events-none overflow-visible"
				>
					<polygon
						ref={lassoRef}
						fill="color-mix(in oklch, var(--primary) 10%, transparent)"
						stroke="var(--primary)"
						strokeWidth={1}
						strokeDasharray="4 3"
					/>
				</svg>
			)}
			{props.interactive !== false && (
				<output
					aria-live="polite"
					className={
						pending
							? "absolute bottom-3 left-16 z-20 rounded bg-card px-2 py-1 text-xs text-muted-foreground pointer-events-none"
							: "sr-only"
					}
				>
					{pending ? "Saving connection…" : announcement}
				</output>
			)}
		</div>
	);
}
