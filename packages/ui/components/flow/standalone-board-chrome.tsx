"use client";

import { useTranslation } from "@flow-like/locales";
import type { Node } from "@xyflow/react";
import {
	LockKeyhole,
	Maximize,
	Minus,
	Plus,
	ShareIcon,
	UnlockKeyhole,
} from "lucide-react";
import { useEffect, useRef, useSyncExternalStore } from "react";
import { canvasGridMultiplier } from "./adaptive-canvas-background";
import { useBoardRuntime } from "./board-runtime";
import { miniMapProjection, sameMiniMapRectangles } from "./egui-mini-map";

export function StandaloneBoardControls({
	onScreenshot,
	exporting = false,
}: { onScreenshot: () => void | Promise<void>; exporting?: boolean }) {
	const runtime = useBoardRuntime();
	if (!runtime) throw new Error("The standalone board needs a board runtime.");
	const { t } = useTranslation("flow");
	const interactive = useSyncExternalStore(
		runtime.store.subscribe,
		() => runtime.store.getState().nodesDraggable,
		() => true,
	);
	const zoomLimit = useSyncExternalStore(
		runtime.store.subscribe,
		() => {
			const { transform, minZoom, maxZoom } = runtime.store.getState();
			if (transform[2] <= minZoom) return "min";
			return transform[2] >= maxZoom ? "max" : undefined;
		},
		() => undefined,
	);
	const buttonClass =
		"react-flow__controls-button flex size-8 items-center justify-center border-b border-border bg-card text-foreground hover:bg-accent disabled:opacity-50 [&>svg]:size-4";
	return (
		<div
			className="react-flow__controls absolute bottom-4 left-4 z-50 overflow-hidden rounded-md border border-border shadow-sm"
			data-board-canvas-controls
		>
			<button
				type="button"
				className={`${buttonClass} react-flow__controls-zoomin`}
				title="zoom in"
				aria-label="zoom in"
				disabled={zoomLimit === "max"}
				onClick={() => void runtime.flow.zoomIn()}
			>
				<Plus />
			</button>
			<button
				type="button"
				className={`${buttonClass} react-flow__controls-zoomout`}
				title="zoom out"
				aria-label="zoom out"
				disabled={zoomLimit === "min"}
				onClick={() => void runtime.flow.zoomOut()}
			>
				<Minus />
			</button>
			<button
				type="button"
				className={`${buttonClass} react-flow__controls-fitview`}
				title="fit view"
				aria-label="fit view"
				onClick={() => void runtime.flow.fitView()}
			>
				<Maximize />
			</button>
			<button
				type="button"
				className={`${buttonClass} react-flow__controls-interactive`}
				title="toggle interactivity"
				aria-label="toggle interactivity"
				aria-pressed={!interactive}
				onClick={() =>
					runtime.store.setState({
						nodesDraggable: !interactive,
						nodesConnectable: !interactive,
						elementsSelectable: !interactive,
					})
				}
			>
				{interactive ? <UnlockKeyhole /> : <LockKeyhole />}
			</button>
			<button
				type="button"
				className={`${buttonClass} border-b-0`}
				aria-label={t("captureScreenshot", "Capture screenshot")}
				aria-busy={exporting}
				disabled={exporting}
				onClick={() => void onScreenshot()}
			>
				<ShareIcon />
			</button>
		</div>
	);
}

/** Sparse world-aligned grid, with no graph component or React viewport subscription. */
export function StandaloneBoardBackground({
	insideLayer,
}: { insideLayer: boolean }) {
	const runtime = useBoardRuntime();
	const element = useRef<HTMLDivElement>(null);
	useEffect(() => {
		const root = runtime?.store.getState().domNode;
		const target = element.current;
		if (!runtime || !root || !target) return;
		const previousColor = root.style.backgroundColor;
		root.style.backgroundColor =
			"color-mix(in oklch, var(--background) 80%, transparent)";
		const draw = () => {
			const [x, y, zoom] = runtime.store.getState().transform;
			const step = 12 * canvasGridMultiplier(zoom) * zoom;
			const radius = Math.max(0.35, zoom * 0.5);
			target.style.backgroundSize = `${step}px ${step}px`;
			if (insideLayer) {
				target.style.backgroundPosition = `${x}px ${y}px`;
				target.style.backgroundImage = `linear-gradient(to right, color-mix(in oklch, var(--foreground) 5%, transparent) ${Math.max(0.5, zoom)}px, transparent 0),linear-gradient(to bottom, color-mix(in oklch, var(--foreground) 5%, transparent) ${Math.max(0.5, zoom)}px, transparent 0)`;
			} else {
				target.style.backgroundPosition = `${x + 6 * zoom - step / 2}px ${y + 6 * zoom - step / 2}px`;
				target.style.backgroundImage = `radial-gradient(circle, color-mix(in oklch, var(--foreground) 20%, transparent) ${radius}px, transparent ${radius + 0.1}px)`;
			}
		};
		draw();
		const unsubscribe = runtime.store.subscribe((state, old) => {
			if (state.transform !== old.transform) draw();
		});
		return () => {
			unsubscribe();
			root.style.backgroundColor = previousColor;
		};
	}, [runtime, insideLayer]);
	return (
		<div
			ref={element}
			data-board-background
			aria-hidden="true"
			style={{
				position: "absolute",
				inset: 0,
				zIndex: -2147483647,
				pointerEvents: "none",
			}}
		/>
	);
}

interface Bounds {
	x: number;
	y: number;
	width: number;
	height: number;
}
interface MiniMapRect extends Bounds {
	color: string;
}

/** Graph rectangles are cached; camera updates only composite the bitmap and viewport mask. */
export function StandaloneBoardMiniMap({
	nodes,
	nodeColor,
	themeKey,
}: {
	nodes: readonly Node[];
	nodeColor: (node: Node) => string;
	themeKey?: unknown;
}) {
	const runtime = useBoardRuntime();
	const svg = useRef<SVGSVGElement>(null);
	const pixels = useRef<HTMLCanvasElement>(null);
	const mask = useRef<SVGPathElement>(null);
	const latest = useRef({ nodes, nodeColor, themeKey });
	latest.current = { nodes, nodeColor, themeKey };
	const refresh = useRef<() => void>(() => {});
	// biome-ignore lint/correctness/useExhaustiveDependencies: Graph and theme changes invalidate cached node pixels.
	useEffect(() => {
		refresh.current();
	}, [nodes, nodeColor, themeKey]);
	useEffect(() => {
		const element = svg.current;
		const display = pixels.current;
		const displayContext = display?.getContext("2d");
		if (!runtime || !element || !display || !displayContext) return;
		let frame = 0;
		let dirty = true;
		let bounds: Bounds | undefined;
		let projection = miniMapProjection(undefined, {
			x: 0,
			y: 0,
			width: 1,
			height: 1,
		});
		let dragging:
			| {
					id: number;
					x: number;
					y: number;
					viewport: ReturnType<typeof runtime.flow.getViewport>;
			  }
			| undefined;
		let rectangles: MiniMapRect[] | undefined;
		let previousTheme: unknown;
		const bitmap = document.createElement("canvas");
		const context = bitmap.getContext("2d");
		const probe = document.createElement("span");
		probe.style.cssText =
			"position:absolute;visibility:hidden;pointer-events:none";
		element.parentElement?.append(probe);
		const redraw = () => {
			if (!context) return;
			const lookup = runtime.store.getState().nodeLookup;
			const next = latest.current.nodes.flatMap((node) => {
				if (node.hidden) return [];
				const internal = lookup.get(node.id);
				const width =
					internal?.measured.width ?? node.measured?.width ?? node.width ?? 0;
				const height =
					internal?.measured.height ??
					node.measured?.height ??
					node.height ??
					0;
				return width > 0 && height > 0
					? [
							{
								...(internal?.internals.positionAbsolute ?? node.position),
								width,
								height,
								color: latest.current.nodeColor(node) || "#e2e2e2",
							},
						]
					: [];
			});
			if (
				rectangles &&
				previousTheme === latest.current.themeKey &&
				sameMiniMapRectangles(rectangles, next)
			)
				return;
			rectangles = next;
			previousTheme = latest.current.themeKey;
			if (!next.length) {
				bounds = undefined;
				return;
			}
			let left = Number.POSITIVE_INFINITY;
			let top = Number.POSITIVE_INFINITY;
			let right = Number.NEGATIVE_INFINITY;
			let bottom = Number.NEGATIVE_INFINITY;
			for (const rect of next) {
				left = Math.min(left, rect.x);
				top = Math.min(top, rect.y);
				right = Math.max(right, rect.x + rect.width);
				bottom = Math.max(bottom, rect.y + rect.height);
			}
			bounds = { x: left, y: top, width: right - left, height: bottom - top };
			const scale = 1024 / Math.max(bounds.width, bounds.height, 1);
			bitmap.width = Math.max(1, Math.ceil(bounds.width * scale));
			bitmap.height = Math.max(1, Math.ceil(bounds.height * scale));
			context.setTransform(scale, 0, 0, scale, -left * scale, -top * scale);
			const colors = new Map<string, string>();
			for (const rect of next) {
				let color = colors.get(rect.color);
				if (!color) {
					probe.style.color = rect.color;
					color = getComputedStyle(probe).color;
					colors.set(rect.color, color);
				}
				context.fillStyle = color;
				context.beginPath();
				context.roundRect(rect.x, rect.y, rect.width, rect.height, 5);
				context.fill();
			}
		};
		const paint = () => {
			frame = 0;
			if (dirty) {
				dirty = false;
				redraw();
			}
			const state = runtime.store.getState();
			const [x, y, zoom] = state.transform;
			const view = {
				x: -x / zoom,
				y: -y / zoom,
				width: state.width / zoom,
				height: state.height / zoom,
			};
			projection = miniMapProjection(bounds, view);
			const dpr = window.devicePixelRatio || 1;
			if (
				display.width !== Math.round(200 * dpr) ||
				display.height !== Math.round(150 * dpr)
			) {
				display.width = Math.round(200 * dpr);
				display.height = Math.round(150 * dpr);
			}
			displayContext.setTransform(dpr, 0, 0, dpr, 0, 0);
			displayContext.clearRect(0, 0, 200, 150);
			if (bounds) {
				const ratio = Math.min(200 / projection.width, 150 / projection.height);
				displayContext.drawImage(
					bitmap,
					(200 - projection.width * ratio) / 2 +
						(bounds.x - projection.x) * ratio,
					(150 - projection.height * ratio) / 2 +
						(bounds.y - projection.y) * ratio,
					bounds.width * ratio,
					bounds.height * ratio,
				);
			}
			element.setAttribute(
				"viewBox",
				`${projection.x} ${projection.y} ${projection.width} ${projection.height}`,
			);
			mask.current?.setAttribute(
				"d",
				`M${projection.x},${projection.y}h${projection.width}v${projection.height}h${-projection.width}z M${view.x},${view.y}h${view.width}v${view.height}h${-view.width}z`,
			);
		};
		const schedule = () => {
			if (!frame) frame = requestAnimationFrame(paint);
		};
		refresh.current = () => {
			dirty = true;
			schedule();
		};
		const point = (event: PointerEvent | WheelEvent) => {
			const matrix = element.getScreenCTM();
			if (!matrix) return { x: projection.x, y: projection.y };
			const p = new DOMPoint(event.clientX, event.clientY).matrixTransform(
				matrix.inverse(),
			);
			return { x: p.x, y: p.y };
		};
		const down = (event: PointerEvent) => {
			if (event.button !== 0) return;
			event.preventDefault();
			event.stopPropagation();
			const p = point(event);
			const state = runtime.store.getState();
			const view = runtime.flow.getViewport();
			const inside =
				p.x >= -view.x / view.zoom &&
				p.x <= (state.width - view.x) / view.zoom &&
				p.y >= -view.y / view.zoom &&
				p.y <= (state.height - view.y) / view.zoom;
			if (!inside)
				void runtime.setViewport(
					{
						x: state.width / 2 - p.x * view.zoom,
						y: state.height / 2 - p.y * view.zoom,
						zoom: view.zoom,
					},
					{ notify: false },
				);
			dragging = {
				id: event.pointerId,
				...p,
				viewport: runtime.flow.getViewport(),
			};
			element.setPointerCapture(event.pointerId);
			state.onMoveStart?.(event, runtime.flow.getViewport());
		};
		const move = (event: PointerEvent) => {
			if (!dragging || dragging.id !== event.pointerId) return;
			event.preventDefault();
			const p = point(event);
			void runtime.setViewport(
				{
					...dragging.viewport,
					x: dragging.viewport.x - (p.x - dragging.x) * dragging.viewport.zoom,
					y: dragging.viewport.y - (p.y - dragging.y) * dragging.viewport.zoom,
				},
				{ notify: false },
			);
		};
		const up = (event: PointerEvent) => {
			if (!dragging || dragging.id !== event.pointerId) return;
			dragging = undefined;
			if (element.hasPointerCapture(event.pointerId))
				element.releasePointerCapture(event.pointerId);
			runtime.store.getState().onMoveEnd?.(event, runtime.flow.getViewport());
		};
		const wheel = (event: WheelEvent) => {
			event.preventDefault();
			event.stopPropagation();
			const state = runtime.store.getState();
			const viewport = runtime.flow.getViewport();
			const zoom = Math.max(
				state.minZoom,
				Math.min(
					state.maxZoom,
					viewport.zoom * Math.exp(-event.deltaY * 0.002),
				),
			);
			void runtime.flow.setCenter(
				(state.width / 2 - viewport.x) / viewport.zoom,
				(state.height / 2 - viewport.y) / viewport.zoom,
				{ zoom },
			);
		};
		element.addEventListener("pointerdown", down);
		element.addEventListener("pointermove", move);
		element.addEventListener("pointerup", up);
		element.addEventListener("pointercancel", up);
		element.addEventListener("wheel", wheel, { passive: false });
		const unsubscribe = runtime.store.subscribe((state, previous) => {
			if (
				state.nodes !== previous.nodes ||
				state.geometryRevision !== previous.geometryRevision
			)
				dirty = true;
			if (
				dirty ||
				state.transform !== previous.transform ||
				state.width !== previous.width ||
				state.height !== previous.height
			)
				schedule();
		});
		schedule();
		return () => {
			unsubscribe();
			cancelAnimationFrame(frame);
			probe.remove();
			refresh.current = () => {};
			element.removeEventListener("pointerdown", down);
			element.removeEventListener("pointermove", move);
			element.removeEventListener("pointerup", up);
			element.removeEventListener("pointercancel", up);
			element.removeEventListener("wheel", wheel);
		};
	}, [runtime]);
	return (
		<div
			className="react-flow__minimap absolute bottom-4 right-4 z-50 h-[150px] w-[200px] overflow-hidden rounded-md border border-border bg-background"
			data-testid="rf__minimap"
			data-board-canvas-controls
		>
			<canvas
				ref={pixels}
				width={200}
				height={150}
				tabIndex={-1}
				aria-hidden="true"
				className="absolute inset-0 h-[150px] w-[200px] pointer-events-none"
			/>
			<svg
				ref={svg}
				width={200}
				height={150}
				className="react-flow__minimap-svg relative touch-none"
				role="img"
				aria-label="Mini Map"
			>
				<title>Mini Map</title>
				<path
					ref={mask}
					className="react-flow__minimap-mask"
					fill="rgb(127 127 127 / 0.15)"
					fillRule="evenodd"
					pointerEvents="none"
				/>
			</svg>
		</div>
	);
}
