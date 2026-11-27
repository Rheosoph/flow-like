"use client";

import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import {
	type ChartView,
	edgeNudge,
	fitView,
	panBy,
	topView,
	wheelPixels,
	wheelZoomFactor,
	zoomAt,
} from "./org-chart-viewport";

const GRID_PX = 18;
const DRAG_SLOP_PX = 4;
const EDGE_PX = 48;
const EDGE_STEP_PX = 14;
const FOCUS_MARGIN_PX = 24;
const BUTTON_ZOOM = 1.2;
const NOT_PANNABLE =
	"button, a, input, textarea, select, [draggable='true'], [role='menuitem'], [contenteditable='true']";

type Point = { x: number; y: number };
type GestureEvent = Event & { scale: number; clientX: number; clientY: number };

export interface ChartZoom {
	subscribe: (listener: () => void) => () => void;
	get: () => number;
}

export function useChartZoom(zoom: ChartZoom): number {
	return useSyncExternalStore(zoom.subscribe, zoom.get, zoom.get);
}

function zoomStore() {
	let value = 1;
	const listeners = new Set<() => void>();
	return {
		get: () => value,
		set(next: number) {
			if (next === value) return;
			value = next;
			for (const listener of listeners) listener();
		},
		subscribe(listener: () => void) {
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
			};
		},
	};
}

function isFocusVisible(element: Element) {
	try {
		return element.matches(":focus-visible");
	} catch {
		return false;
	}
}

function focusNudge(viewport: DOMRect, target: DOMRect) {
	const span = (lo: number, hi: number, min: number, max: number) => {
		if (lo < min + FOCUS_MARGIN_PX) return min + FOCUS_MARGIN_PX - lo;
		if (hi > max - FOCUS_MARGIN_PX) return max - FOCUS_MARGIN_PX - hi;
		return 0;
	};
	return {
		dx: span(target.left, target.right, viewport.left, viewport.right),
		dy: span(target.top, target.bottom, viewport.top, viewport.bottom),
	};
}

export function useChartViewport(dragging: boolean) {
	const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
	const [content, setContent] = useState<HTMLDivElement | null>(null);
	const view = useRef<ChartView>({ x: 0, y: 0, k: 1 });
	const draggingRef = useRef(dragging);
	const zoom = useMemo(zoomStore, []);

	useEffect(() => {
		draggingRef.current = dragging;
	}, [dragging]);

	const apply = useCallback(
		(next: ChartView) => {
			view.current = next;
			if (content) {
				content.style.transform = `translate(${next.x}px, ${next.y}px) scale(${next.k})`;
			}
			if (viewport) {
				const grid = GRID_PX * next.k;
				viewport.style.backgroundPosition = `${next.x}px ${next.y}px`;
				viewport.style.backgroundSize = `${grid}px ${grid}px`;
			}
			zoom.set(next.k);
		},
		[content, viewport, zoom],
	);

	const measure = useCallback(() => {
		if (!viewport || !content) return null;
		const box = { width: viewport.clientWidth, height: viewport.clientHeight };
		const size = { width: content.offsetWidth, height: content.offsetHeight };
		if (!box.width || !box.height || !size.width || !size.height) return null;
		return { box, size };
	}, [viewport, content]);

	useLayoutEffect(() => {
		const sizes = measure();
		if (sizes) apply(topView(sizes.box, sizes.size));
	}, [measure, apply]);

	const fit = useCallback(() => {
		const sizes = measure();
		if (sizes) apply(fitView(sizes.box, sizes.size));
	}, [measure, apply]);

	const zoomBy = useCallback(
		(factor: number) => {
			if (!viewport) return;
			const current = view.current;
			apply(
				zoomAt(
					current,
					current.k * factor,
					viewport.clientWidth / 2,
					viewport.clientHeight / 2,
				),
			);
		},
		[viewport, apply],
	);

	useEffect(() => {
		if (!viewport) return;
		const doc = viewport.ownerDocument;
		const pointers = new Map<number, Point>();
		let pan: { id: number; x: number; y: number; moved: boolean } | null = null;
		let pinch: { distance: number; k: number; mid: Point } | null = null;
		let gesture: number | null = null;
		let swallowClick = false;

		const local = (clientX: number, clientY: number): Point => {
			const rect = viewport.getBoundingClientRect();
			return { x: clientX - rect.left, y: clientY - rect.top };
		};
		const pair = () => {
			const [a, b] = [...pointers.values()];
			return {
				distance: Math.max(1, Math.hypot(a.x - b.x, a.y - b.y)),
				mid: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 },
			};
		};
		const endPan = () => {
			if (pan?.moved) swallowClick = true;
			pan = null;
			delete viewport.dataset.panning;
		};
		const onPointerUp = (event: PointerEvent) => {
			if (!pointers.delete(event.pointerId)) return;
			if (pinch && pointers.size < 2) {
				pinch = null;
				swallowClick = true;
			}
			if (pan?.id === event.pointerId) endPan();
		};

		const onWheel = (event: WheelEvent) => {
			event.preventDefault();
			const { dx, dy } = wheelPixels(event, viewport.clientHeight);
			if (!(event.ctrlKey || event.metaKey)) {
				apply(panBy(view.current, -dx, -dy));
				return;
			}
			if (gesture !== null) return;
			const point = local(event.clientX, event.clientY);
			apply(
				zoomAt(
					view.current,
					view.current.k * wheelZoomFactor(dy),
					point.x,
					point.y,
				),
			);
		};

		const onGestureStart = (event: Event) => {
			event.preventDefault();
			gesture = view.current.k;
		};
		const onGestureChange = (event: Event) => {
			event.preventDefault();
			if (gesture === null) return;
			const { scale, clientX, clientY } = event as GestureEvent;
			const point = local(clientX, clientY);
			apply(zoomAt(view.current, gesture * scale, point.x, point.y));
		};
		const onGestureEnd = (event: Event) => {
			event.preventDefault();
			gesture = null;
		};

		const onPointerDown = (event: PointerEvent) => {
			swallowClick = false;
			pointers.set(event.pointerId, local(event.clientX, event.clientY));
			if (pointers.size === 2) {
				endPan();
				pinch = { ...pair(), k: view.current.k };
				return;
			}
			if (pointers.size > 2) return;
			const middle = event.button === 1;
			const target = event.target as Element | null;
			if (!middle && (event.button !== 0 || target?.closest(NOT_PANNABLE))) {
				return;
			}
			pan = {
				id: event.pointerId,
				x: event.clientX,
				y: event.clientY,
				moved: false,
			};
		};

		const onPointerMove = (event: PointerEvent) => {
			if (!pointers.has(event.pointerId)) return;
			if (event.pointerType === "mouse" && event.buttons === 0) {
				onPointerUp(event);
				return;
			}
			pointers.set(event.pointerId, local(event.clientX, event.clientY));
			if (pinch && pointers.size >= 2) {
				const { distance, mid } = pair();
				const zoomed = zoomAt(
					view.current,
					(pinch.k * distance) / pinch.distance,
					mid.x,
					mid.y,
				);
				apply(panBy(zoomed, mid.x - pinch.mid.x, mid.y - pinch.mid.y));
				pinch.mid = mid;
				return;
			}
			if (pan?.id !== event.pointerId) return;
			const dx = event.clientX - pan.x;
			const dy = event.clientY - pan.y;
			if (!pan.moved && Math.hypot(dx, dy) < DRAG_SLOP_PX) return;
			pan.moved = true;
			pan.x = event.clientX;
			pan.y = event.clientY;
			viewport.dataset.panning = "true";
			apply(panBy(view.current, dx, dy));
		};

		const onClickCapture = (event: MouseEvent) => {
			if (!swallowClick) return;
			swallowClick = false;
			event.preventDefault();
			event.stopPropagation();
		};

		const onMouseDown = (event: MouseEvent) => {
			if (event.button === 1) event.preventDefault();
		};

		const onDragOver = (event: DragEvent) => {
			if (!draggingRef.current) return;
			const rect = viewport.getBoundingClientRect();
			const dx =
				edgeNudge(event.clientX - rect.left, EDGE_PX, EDGE_STEP_PX) -
				edgeNudge(rect.right - event.clientX, EDGE_PX, EDGE_STEP_PX);
			const dy =
				edgeNudge(event.clientY - rect.top, EDGE_PX, EDGE_STEP_PX) -
				edgeNudge(rect.bottom - event.clientY, EDGE_PX, EDGE_STEP_PX);
			if (dx || dy) apply(panBy(view.current, dx, dy));
		};

		const onFocusIn = (event: FocusEvent) => {
			const target = event.target as Element | null;
			if (!target || target === viewport || !isFocusVisible(target)) return;
			const { dx, dy } = focusNudge(
				viewport.getBoundingClientRect(),
				target.getBoundingClientRect(),
			);
			if (dx || dy) apply(panBy(view.current, dx, dy));
		};

		const listeners: [EventTarget, string, EventListener, boolean?][] = [
			[viewport, "wheel", onWheel as EventListener],
			[viewport, "gesturestart", onGestureStart],
			[viewport, "gesturechange", onGestureChange],
			[viewport, "gestureend", onGestureEnd],
			[viewport, "pointerdown", onPointerDown as EventListener],
			[viewport, "mousedown", onMouseDown as EventListener],
			[viewport, "click", onClickCapture as EventListener, true],
			[viewport, "dragover", onDragOver as EventListener],
			[viewport, "focusin", onFocusIn as EventListener],
			[doc, "pointermove", onPointerMove as EventListener],
			[doc, "pointerup", onPointerUp as EventListener],
			[doc, "pointercancel", onPointerUp as EventListener],
		];
		for (const [target, type, listener, capture] of listeners) {
			target.addEventListener(type, listener, {
				passive: false,
				capture: capture ?? false,
			});
		}
		return () => {
			for (const [target, type, listener, capture] of listeners) {
				target.removeEventListener(type, listener, {
					capture: capture ?? false,
				});
			}
			delete viewport.dataset.panning;
		};
	}, [viewport, apply]);

	return {
		viewportRef: setViewport,
		contentRef: setContent,
		zoom,
		zoomIn: useCallback(() => zoomBy(BUTTON_ZOOM), [zoomBy]),
		zoomOut: useCallback(() => zoomBy(1 / BUTTON_ZOOM), [zoomBy]),
		fit,
	};
}
