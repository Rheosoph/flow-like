export interface GuideBox {
	x: number;
	y: number;
	width: number;
	height: number;
}

/** A pin on a dragged node and the pin it is wired to, both in flow space. */
export interface PinGuide {
	x: number;
	y: number;
	targetX: number;
	targetY: number;
}

/** `axis: "x"` is a vertical line at x = `at`; `"y"` a horizontal one. */
export interface HelperLine {
	axis: "x" | "y";
	at: number;
	from: number;
	to: number;
	pin?: boolean;
}

export interface GuideSnap {
	dx: number;
	dy: number;
	lines: HelperLine[];
}

export interface GuideSnapOptions {
	threshold: number;
	axes?: { x: boolean; y: boolean };
	pins?: readonly PinGuide[];
}

type Axis = "x" | "y";

const ANCHORS = [0, 0.5, 1] as const;
const ALIGNED_EPSILON = 0.5;

const start = (box: GuideBox, axis: Axis) => (axis === "x" ? box.x : box.y);
const extent = (box: GuideBox, axis: Axis) =>
	axis === "x" ? box.width : box.height;
const anchor = (box: GuideBox, axis: Axis, at: number) =>
	start(box, axis) + extent(box, axis) * at;
const across = (axis: Axis): Axis => (axis === "x" ? "y" : "x");

export function unionBox(boxes: readonly GuideBox[]): GuideBox | undefined {
	if (boxes.length === 0) return undefined;
	let minX = Number.POSITIVE_INFINITY;
	let minY = Number.POSITIVE_INFINITY;
	let maxX = Number.NEGATIVE_INFINITY;
	let maxY = Number.NEGATIVE_INFINITY;
	for (const box of boxes) {
		minX = Math.min(minX, box.x);
		minY = Math.min(minY, box.y);
		maxX = Math.max(maxX, box.x + box.width);
		maxY = Math.max(maxY, box.y + box.height);
	}
	return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

export function gapDistance(a: GuideBox, b: GuideBox): number {
	const dx = Math.max(0, b.x - (a.x + a.width), a.x - (b.x + b.width));
	const dy = Math.max(0, b.y - (a.y + a.height), a.y - (b.y + b.height));
	return Math.hypot(dx, dy);
}

const intersects = (a: GuideBox, b: GuideBox) =>
	a.x <= b.x + b.width &&
	b.x <= a.x + a.width &&
	a.y <= b.y + b.height &&
	b.y <= a.y + a.height;

const contains = (outer: GuideBox, inner: GuideBox) =>
	outer.x <= inner.x &&
	outer.y <= inner.y &&
	outer.x + outer.width >= inner.x + inner.width &&
	outer.y + outer.height >= inner.y + inner.height;

/**
 * The boxes worth aligning to: on screen, not nested with the dragged box
 * (flush against a comment frame's border is never the intent), and either
 * among the `limit` nearest or wired to the drag. A dense board would
 * otherwise offer a snap target every few pixels.
 */
export function pickReferences(
	candidates: readonly GuideBox[],
	moving: GuideBox,
	{
		visible,
		limit,
		always,
	}: {
		visible: GuideBox;
		limit: number;
		always?: ReadonlySet<GuideBox>;
	},
): GuideBox[] {
	const nearest: { box: GuideBox; distance: number }[] = [];
	const picked: GuideBox[] = [];
	for (const box of candidates) {
		if (!intersects(box, visible)) continue;
		if (contains(box, moving) || contains(moving, box)) continue;
		if (always?.has(box)) {
			picked.push(box);
			continue;
		}
		const distance = gapDistance(box, moving);
		const worst = nearest.at(-1);
		if (nearest.length >= limit && (!worst || distance >= worst.distance))
			continue;
		let index = nearest.length;
		while (index > 0 && nearest[index - 1].distance > distance) index--;
		nearest.splice(index, 0, { box, distance });
		if (nearest.length > limit) nearest.pop();
	}
	for (const { box } of nearest) picked.push(box);
	return picked;
}

function boxOffset(
	axis: Axis,
	moving: GuideBox,
	references: readonly GuideBox[],
	threshold: number,
): number | undefined {
	let best: number | undefined;
	let bestDistance = Number.POSITIVE_INFINITY;
	for (const reference of references) {
		const distance = gapDistance(reference, moving);
		for (const at of ANCHORS) {
			const offset = anchor(reference, axis, at) - anchor(moving, axis, at);
			if (Math.abs(offset) > threshold) continue;
			const closer =
				best === undefined ||
				Math.abs(offset) < Math.abs(best) - ALIGNED_EPSILON ||
				(Math.abs(offset) <= Math.abs(best) + ALIGNED_EPSILON &&
					distance < bestDistance);
			if (!closer) continue;
			best = offset;
			bestDistance = distance;
		}
	}
	return best;
}

function pinOffset(
	pins: readonly PinGuide[],
	threshold: number,
): number | undefined {
	let best: number | undefined;
	for (const pin of pins) {
		const offset = pin.targetY - pin.y;
		if (Math.abs(offset) > threshold) continue;
		if (best === undefined || Math.abs(offset) < Math.abs(best)) best = offset;
	}
	return best;
}

function boxLines(
	axis: Axis,
	moving: GuideBox,
	references: readonly GuideBox[],
): HelperLine[] {
	const other = across(axis);
	const lines: HelperLine[] = [];
	for (const at of ANCHORS) {
		const value = anchor(moving, axis, at);
		let nearest: GuideBox | undefined;
		let nearestDistance = Number.POSITIVE_INFINITY;
		for (const reference of references) {
			if (Math.abs(anchor(reference, axis, at) - value) > ALIGNED_EPSILON)
				continue;
			const distance = gapDistance(reference, moving);
			if (distance >= nearestDistance) continue;
			nearest = reference;
			nearestDistance = distance;
		}
		if (!nearest) continue;
		lines.push({
			axis,
			at: value,
			from: Math.min(start(moving, other), start(nearest, other)),
			to: Math.max(anchor(moving, other, 1), anchor(nearest, other, 1)),
		});
	}
	return lines;
}

function pinLines(pins: readonly PinGuide[], dx: number, dy: number) {
	const lines: HelperLine[] = [];
	for (const pin of pins) {
		if (Math.abs(pin.y + dy - pin.targetY) > ALIGNED_EPSILON) continue;
		lines.push({
			axis: "y",
			at: pin.targetY,
			from: Math.min(pin.x + dx, pin.targetX),
			to: Math.max(pin.x + dx, pin.targetX),
			pin: true,
		});
	}
	return lines;
}

/**
 * Snap `moving` to the references like-for-like (start, centre, end) on each
 * free axis. A pin that would straighten its wire outranks box alignment on
 * the y axis: in a node graph a level wire matters more than level edges.
 */
export function snapToGuides(
	moving: GuideBox,
	references: readonly GuideBox[],
	{ threshold, axes = { x: true, y: true }, pins = [] }: GuideSnapOptions,
): GuideSnap {
	const dx = axes.x ? (boxOffset("x", moving, references, threshold) ?? 0) : 0;
	const dy = axes.y
		? (pinOffset(pins, threshold) ??
			boxOffset("y", moving, references, threshold) ??
			0)
		: 0;
	const snapped = { ...moving, x: moving.x + dx, y: moving.y + dy };
	const lines = [
		...(axes.x ? boxLines("x", snapped, references) : []),
		...(axes.y
			? [...pinLines(pins, dx, dy), ...boxLines("y", snapped, references)]
			: []),
	];
	return { dx, dy, lines };
}
