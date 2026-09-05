import type { IHomeWidget } from "./types";

export interface HomeDragRect {
	id: string;
	left: number;
	top: number;
	right: number;
	bottom: number;
}
export interface HomeDragPoint {
	x: number;
	y: number;
}
const contains = (rect: Omit<HomeDragRect, "id">, point: HomeDragPoint) =>
	point.x >= rect.left &&
	point.x <= rect.right &&
	point.y >= rect.top &&
	point.y <= rect.bottom;

/** Resolve an insertion slot from visible widget bounds, including the existing placeholder. */
export function homeInsertionIndex(
	widgets: IHomeWidget[],
	movingId: string,
	point: HomeDragPoint,
	canvas: Omit<HomeDragRect, "id">,
	rects: HomeDragRect[],
): number | null {
	if (!contains(canvas, point)) return null;
	const currentIndex = widgets.findIndex((widget) => widget.id === movingId);
	const placeholder = rects.find((rect) => rect.id === movingId);
	if (placeholder && contains(placeholder, point)) return currentIndex;
	const remaining = widgets.filter((widget) => widget.id !== movingId);
	const candidates = rects.filter(
		(rect) =>
			rect.id !== movingId && remaining.some((widget) => widget.id === rect.id),
	);
	if (!candidates.length) return 0;
	if (point.y > Math.max(...candidates.map((rect) => rect.bottom)))
		return remaining.length;
	const distance = (rect: HomeDragRect) =>
		Math.hypot(
			Math.max(rect.left - point.x, 0, point.x - rect.right),
			Math.max(rect.top - point.y, 0, point.y - rect.bottom),
		);
	const target = [...candidates].sort((a, b) => distance(a) - distance(b))[0];
	const width = target.right - target.left;
	const height = target.bottom - target.top;
	const vertical =
		width >= (canvas.right - canvas.left) * 0.72 ||
		point.y < target.top + Math.min(28, height * 0.2) ||
		point.y > target.bottom - Math.min(28, height * 0.2);
	const after = vertical
		? point.y > (target.top + target.bottom) / 2
		: point.x > (target.left + target.right) / 2;
	return (
		remaining.findIndex((widget) => widget.id === target.id) + (after ? 1 : 0)
	);
}

export function insertHomeWidget(
	widgets: IHomeWidget[],
	widget: IHomeWidget,
	index: number,
) {
	const next = widgets.filter((item) => item.id !== widget.id);
	next.splice(Math.max(0, Math.min(index, next.length)), 0, widget);
	return next;
}
