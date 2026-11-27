export interface ChartView {
	readonly x: number;
	readonly y: number;
	readonly k: number;
}

export interface Size {
	readonly width: number;
	readonly height: number;
}

export const ZOOM_MIN = 0.2;
export const ZOOM_MAX = 2;

const LINE_PX = 16;
const WHEEL_ZOOM_RATE = 0.0075;
const WHEEL_ZOOM_CAP_PX = 40;

export function clampZoom(k: number): number {
	return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, k));
}

export function panBy(view: ChartView, dx: number, dy: number): ChartView {
	return { ...view, x: view.x + dx, y: view.y + dy };
}

export function zoomAt(
	view: ChartView,
	k: number,
	px: number,
	py: number,
): ChartView {
	const next = clampZoom(k);
	const ratio = next / view.k;
	return {
		k: next,
		x: px - (px - view.x) * ratio,
		y: py - (py - view.y) * ratio,
	};
}

export function fitView(box: Size, content: Size, maxK = 1): ChartView {
	const k = clampZoom(
		Math.min(box.width / content.width, box.height / content.height, maxK),
	);
	return {
		k,
		x: (box.width - content.width * k) / 2,
		y: (box.height - content.height * k) / 2,
	};
}

export function topView(box: Size, content: Size): ChartView {
	const k = clampZoom(Math.min(1, box.width / content.width));
	return { k, x: (box.width - content.width * k) / 2, y: 0 };
}

export function wheelPixels(
	event: Pick<WheelEvent, "deltaX" | "deltaY" | "deltaMode" | "shiftKey">,
	pageHeight: number,
): { dx: number; dy: number } {
	const unit =
		event.deltaMode === 1 ? LINE_PX : event.deltaMode === 2 ? pageHeight : 1;
	const dx = event.deltaX * unit;
	const dy = event.deltaY * unit;
	return event.shiftKey && dx === 0 ? { dx: dy, dy: 0 } : { dx, dy };
}

export function wheelZoomFactor(dy: number): number {
	const capped = Math.max(-WHEEL_ZOOM_CAP_PX, Math.min(WHEEL_ZOOM_CAP_PX, dy));
	return Math.exp(-capped * WHEEL_ZOOM_RATE);
}

export function edgeNudge(offset: number, edge: number, step: number): number {
	if (offset >= edge) return 0;
	return Math.ceil(step * (1 - Math.max(0, offset) / edge));
}
