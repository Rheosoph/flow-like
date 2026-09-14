import type { Settings } from "sigma/settings";
import type { NodeDisplayData, PartialButFor } from "sigma/types";
import {
	GRAPH_LABEL_LEFT_INSET,
	GRAPH_LABEL_RIGHT_INSET,
	computeViewportLabelPlacement,
} from "./graph-layout";
import { getGraphTheme } from "./theme-colors";

type NodeData = PartialButFor<
	NodeDisplayData,
	"x" | "y" | "size" | "label" | "color"
> & {
	/** Population a hub stands for, pre-formatted with its `≥` bound. */
	badge?: string;
	forceLabel?: boolean;
	highlighted?: boolean;
};

const BADGE_GAP = 6;
const BADGE_PADDING_X = 5;
const BADGE_HEIGHT = 15;
const LABEL_GAP = 6;
const LABEL_CLEARANCE = 3;
const LABEL_CELL_SIZE = 64;

interface LabelBounds {
	left: number;
	right: number;
	top: number;
	bottom: number;
}

interface LabelNodeBounds {
	x: number;
	y: number;
	size: number;
}

interface LabelLayout {
	cells: Map<string, LabelBounds[]>;
	getNodeBounds?: () => readonly LabelNodeBounds[];
}

const labelLayouts = new WeakMap<CanvasRenderingContext2D, LabelLayout>();

/** Sigma's position grid does not account for the width of captions or badges. */
export function resetNodeLabelLayout(
	context: CanvasRenderingContext2D,
	getNodeBounds?: () => readonly LabelNodeBounds[],
): void {
	labelLayouts.set(context, { cells: new Map(), getNodeBounds });
}

function labelCellKeys(bounds: LabelBounds): string[] {
	const keys: string[] = [];
	for (
		let column = Math.floor(bounds.left / LABEL_CELL_SIZE);
		column <= Math.floor(bounds.right / LABEL_CELL_SIZE);
		column += 1
	) {
		for (
			let row = Math.floor(bounds.top / LABEL_CELL_SIZE);
			row <= Math.floor(bounds.bottom / LABEL_CELL_SIZE);
			row += 1
		) {
			keys.push(`${column}:${row}`);
		}
	}
	return keys;
}

function indexLabelBounds(
	cells: Map<string, LabelBounds[]>,
	bounds: LabelBounds,
	keys: readonly string[] = labelCellKeys(bounds),
): void {
	for (const key of keys) {
		const entries = cells.get(key);
		if (entries) entries.push(bounds);
		else cells.set(key, [bounds]);
	}
}

function reserveLabelBounds(
	context: CanvasRenderingContext2D,
	bounds: LabelBounds,
): boolean {
	let layout = labelLayouts.get(context);
	if (!layout) {
		layout = { cells: new Map() };
		labelLayouts.set(context, layout);
	}
	const { cells, getNodeBounds } = layout;
	if (getNodeBounds) {
		// Resolve positions lazily after Sigma updates its camera matrices.
		layout.getNodeBounds = undefined;
		for (const node of getNodeBounds()) {
			if (
				![node.x, node.y, node.size].every(Number.isFinite) ||
				node.size <= 0
			) {
				continue;
			}
			indexLabelBounds(cells, {
				left: node.x - node.size,
				right: node.x + node.size,
				top: node.y - node.size,
				bottom: node.y + node.size,
			});
		}
	}
	const keys = labelCellKeys(bounds);
	for (const key of keys) {
		for (const other of cells.get(key) ?? []) {
			if (
				bounds.left < other.right &&
				bounds.right > other.left &&
				bounds.top < other.bottom &&
				bounds.bottom > other.top
			) {
				return false;
			}
		}
	}
	indexLabelBounds(cells, bounds, keys);
	return true;
}

/** Shortest a caption is ever cut; below this a truncation hides more than it helps. */
const TRUNCATE_MIN_CHARS = 16;
/** Longest caption drawn fully zoomed in. The hover card carries the rest. */
const TRUNCATE_MAX_CHARS = 44;
const ELLIPSIS = "…";

/** Vertical clearance the hover card keeps around its text. */
const HOVER_PADDING_Y = 4;
const HOVER_PADDING_X = 6;
const HOVER_MAX_WIDTH = 420;
const VIEWPORT_PADDING_Y = 4;

/**
 * How many characters of a caption survive at this rendered node size.
 *
 * `data.size` already carries the zoom: it is the on-screen radius, so zooming
 * in reveals more of the caption without any camera plumbing here.
 */
export function labelCharBudget(renderedSize: number): number {
	return Math.max(
		TRUNCATE_MIN_CHARS,
		Math.min(TRUNCATE_MAX_CHARS, Math.round(12 + renderedSize * 1.6)),
	);
}

export function truncateLabel(label: string, renderedSize: number): string {
	const budget = labelCharBudget(renderedSize);
	if (label.length <= budget) return label;
	return `${label.slice(0, budget - 1).trimEnd()}${ELLIPSIS}`;
}

function canvasCssDimension(
	context: CanvasRenderingContext2D,
	axis: "width" | "height",
): number {
	const clientSize =
		axis === "width" ? context.canvas.clientWidth : context.canvas.clientHeight;
	if (clientSize > 0) return clientSize;
	const pixelRatio =
		typeof window !== "undefined" && window.devicePixelRatio > 0
			? window.devicePixelRatio
			: 1;
	return context.canvas[axis] / pixelRatio;
}

/** Last-resort truncation for a caption whose preferred side is too narrow. */
function fitTextToWidth(
	context: CanvasRenderingContext2D,
	label: string,
	maxWidth: number,
): string {
	if (!label || maxWidth <= 0) return "";
	if (context.measureText(label).width <= maxWidth) return label;
	if (context.measureText(ELLIPSIS).width > maxWidth) return "";

	let low = 0;
	let high = label.length;
	while (low < high) {
		const middle = Math.ceil((low + high) / 2);
		const candidate = `${label.slice(0, middle).trimEnd()}${ELLIPSIS}`;
		if (context.measureText(candidate).width <= maxWidth) low = middle;
		else high = middle - 1;
	}
	return low > 0 ? `${label.slice(0, low).trimEnd()}${ELLIPSIS}` : ELLIPSIS;
}

function measureBadgeWidth(
	context: CanvasRenderingContext2D,
	badge: string | undefined,
	size: number,
	font: string,
): number {
	if (!badge) return 0;
	const previousFont = context.font;
	context.font = `600 ${size - 1}px ${font}`;
	const width = context.measureText(badge).width + BADGE_PADDING_X * 2;
	context.font = previousFont;
	return width;
}

/**
 * Labels culled by size pop in the moment a node crosses the threshold; a short
 * alpha ramp just above it turns that pop into a fade. Forced and highlighted
 * labels are exempt because they bypass the threshold.
 */
function labelAlpha(data: NodeData, settings: Settings): number {
	if (data.forceLabel || data.highlighted) return 1;
	const threshold = settings.labelRenderedSizeThreshold;
	if (threshold <= 0) return 1;
	const rampEnd = threshold * 1.35;
	if (data.size >= rampEnd) return 1;
	const t = (data.size - threshold) / (rampEnd - threshold);
	return Math.max(0.35, Math.min(1, 0.35 + t * 0.65));
}

function tracePill(
	context: CanvasRenderingContext2D,
	x: number,
	top: number,
	width: number,
	height: number,
): void {
	const radius = Math.min(width, height) / 2;
	context.beginPath();
	// Hand-rolled rather than roundRect: this runs inside the render loop, where
	// an unsupported call would take the whole canvas down rather than one pill.
	context.moveTo(x + radius, top);
	context.lineTo(x + width - radius, top);
	context.arcTo(x + width, top, x + width, top + radius, radius);
	context.lineTo(x + width, top + height - radius);
	context.arcTo(
		x + width,
		top + height,
		x + width - radius,
		top + height,
		radius,
	);
	context.lineTo(x + radius, top + height);
	context.arcTo(x, top + height, x, top + height - radius, radius);
	context.lineTo(x, top + radius);
	context.arcTo(x, top, x + radius, top, radius);
	context.closePath();
}

function drawBadge(
	context: CanvasRenderingContext2D,
	badge: string,
	x: number,
	y: number,
	size: number,
	font: string,
	alpha: number,
): void {
	const theme = getGraphTheme();
	const [fgR, fgG, fgB] = theme.fgRgb;

	context.font = `600 ${size - 1}px ${font}`;
	context.textAlign = "left";
	const badgeWidth = context.measureText(badge).width + BADGE_PADDING_X * 2;

	tracePill(context, x, y - BADGE_HEIGHT / 2, badgeWidth, BADGE_HEIGHT);
	context.fillStyle = `rgba(${fgR},${fgG},${fgB},${(theme.isDark ? 0.16 : 0.1) * alpha})`;
	context.fill();

	context.fillStyle = `rgba(${fgR},${fgG},${fgB},${0.75 * alpha})`;
	context.fillText(badge, x + BADGE_PADDING_X, y);
}

export function drawNodeLabel(
	context: CanvasRenderingContext2D,
	data: NodeData,
	settings: Settings,
): void {
	// Highlighted nodes receive a full caption on Sigma's hover layer.
	if (!data.label || data.hidden || data.highlighted) return;

	const theme = getGraphTheme();
	const [fgR, fgG, fgB] = theme.fgRgb;

	const size = settings.labelSize;
	const font = settings.labelFont;
	const weight = settings.labelWeight;
	const alpha = labelAlpha(data, settings);
	const preferredLabel = truncateLabel(data.label, data.size);
	const y = data.y;
	const halfHeight = Math.max(size, data.badge ? BADGE_HEIGHT : 0) / 2;
	if (
		y - halfHeight < VIEWPORT_PADDING_Y ||
		y + halfHeight > canvasCssDimension(context, "height") - VIEWPORT_PADDING_Y
	) {
		return;
	}

	context.font = `${weight} ${size}px ${font}`;
	context.textBaseline = "middle";
	const viewportWidth = canvasCssDimension(context, "width");
	const badgeWidth = measureBadgeWidth(context, data.badge, size, font);
	const preferredTextWidth = context.measureText(preferredLabel).width;
	const placement = computeViewportLabelPlacement(
		data.x,
		data.size,
		preferredTextWidth + (data.badge ? BADGE_GAP + badgeWidth : 0),
		viewportWidth,
		{
			gap: LABEL_GAP,
			leftInset: GRAPH_LABEL_LEFT_INSET,
			rightInset: GRAPH_LABEL_RIGHT_INSET,
		},
	);
	const availableWidth = Math.min(
		placement.availableWidth,
		Math.max(
			0,
			viewportWidth - GRAPH_LABEL_LEFT_INSET - GRAPH_LABEL_RIGHT_INSET,
		),
	);
	const badge =
		data.badge && badgeWidth + BADGE_GAP < availableWidth
			? data.badge
			: undefined;
	const label = fitTextToWidth(
		context,
		preferredLabel,
		Math.max(0, availableWidth - (badge ? BADGE_GAP + badgeWidth : 0)),
	);
	if (!label && !badge) return;

	const x =
		placement.side === "right"
			? Math.max(GRAPH_LABEL_LEFT_INSET, data.x + data.size + LABEL_GAP)
			: Math.min(
					viewportWidth - GRAPH_LABEL_RIGHT_INSET,
					data.x - data.size - LABEL_GAP,
				);
	const textWidth = label ? context.measureText(label).width : 0;
	const contentWidth = textWidth + (badge ? BADGE_GAP + badgeWidth : 0);
	const left = placement.side === "right" ? x : x - contentWidth;
	if (
		!reserveLabelBounds(context, {
			left: left - LABEL_CLEARANCE,
			right: left + contentWidth + LABEL_CLEARANCE,
			top: y - halfHeight - LABEL_CLEARANCE,
			bottom: y + halfHeight + LABEL_CLEARANCE,
		})
	) {
		return;
	}
	context.textAlign = placement.side === "right" ? "left" : "right";

	const [bgR, bgG, bgB] = theme.bgRgb;
	if (label) {
		context.strokeStyle = `rgba(${bgR},${bgG},${bgB},${0.85 * alpha})`;
		context.lineWidth = 3;
		context.lineJoin = "round";
		context.strokeText(label, x, y);

		context.fillStyle = `rgba(${fgR},${fgG},${fgB},${alpha})`;
		context.fillText(label, x, y);
	}

	if (!badge) return;
	drawBadge(
		context,
		badge,
		placement.side === "right"
			? x + textWidth + (label ? BADGE_GAP : 0)
			: x - textWidth - (label ? BADGE_GAP : 0) - badgeWidth,
		y,
		size,
		font,
		alpha,
	);
}

/**
 * Hover and selection render above ordinary labels. An opaque card keeps the
 * caption and population badge readable over graph edges.
 */
export function drawNodeHover(
	context: CanvasRenderingContext2D,
	data: NodeData,
	settings: Settings,
): void {
	if (data.hidden) return;
	const theme = getGraphTheme();
	const [fgR, fgG, fgB] = theme.fgRgb;
	const [bgR, bgG, bgB] = theme.bgRgb;

	const { x, y, size } = data;

	context.beginPath();
	context.arc(x, y, size + 4, 0, Math.PI * 2);
	context.fillStyle = theme.isDark
		? `rgba(${fgR},${fgG},${fgB},0.08)`
		: "rgba(0,0,0,0.06)";
	context.fill();

	if (!data.label) return;

	const fontSize = settings.labelSize;
	const font = settings.labelFont;
	context.font = `${settings.labelWeight} ${fontSize}px ${font}`;
	context.textAlign = "left";
	context.textBaseline = "middle";

	const badgeWidth = measureBadgeWidth(context, data.badge, fontSize, font);
	const preferredTextWidth = context.measureText(data.label).width;
	const viewportWidth = canvasCssDimension(context, "width");
	const cardHeight =
		Math.max(fontSize, data.badge ? BADGE_HEIGHT : 0) + HOVER_PADDING_Y * 2;
	const viewportHeight = canvasCssDimension(context, "height");
	if (viewportHeight < cardHeight + VIEWPORT_PADDING_Y * 2) return;
	const cardY = Math.max(
		VIEWPORT_PADDING_Y + cardHeight / 2,
		Math.min(viewportHeight - VIEWPORT_PADDING_Y - cardHeight / 2, y),
	);
	const placement = computeViewportLabelPlacement(
		x,
		size,
		Math.min(
			HOVER_MAX_WIDTH,
			preferredTextWidth +
				HOVER_PADDING_X * 2 +
				(data.badge ? BADGE_GAP + badgeWidth : 0),
		),
		viewportWidth,
		{
			gap: LABEL_GAP,
			leftInset: GRAPH_LABEL_LEFT_INSET,
			rightInset: GRAPH_LABEL_RIGHT_INSET,
		},
	);
	const availableWidth = Math.min(
		HOVER_MAX_WIDTH,
		placement.availableWidth,
		Math.max(
			0,
			viewportWidth - GRAPH_LABEL_LEFT_INSET - GRAPH_LABEL_RIGHT_INSET,
		),
	);
	const badge =
		data.badge && badgeWidth + BADGE_GAP + HOVER_PADDING_X * 2 < availableWidth
			? data.badge
			: undefined;
	const label = fitTextToWidth(
		context,
		data.label,
		Math.max(
			0,
			availableWidth -
				HOVER_PADDING_X * 2 -
				(badge ? BADGE_GAP + badgeWidth : 0),
		),
	);
	if (!label && !badge) return;

	const textWidth = label ? context.measureText(label).width : 0;
	const contentWidth = textWidth + (badge ? BADGE_GAP + badgeWidth : 0);
	const cardWidth = contentWidth + HOVER_PADDING_X * 2;
	const preferredCardLeft =
		placement.side === "right"
			? x + size + LABEL_GAP
			: x - size - LABEL_GAP - cardWidth;
	const cardLeft = Math.max(
		GRAPH_LABEL_LEFT_INSET,
		Math.min(
			viewportWidth - GRAPH_LABEL_RIGHT_INSET - cardWidth,
			preferredCardLeft,
		),
	);
	const textX = cardLeft + HOVER_PADDING_X;

	tracePill(context, cardLeft, cardY - cardHeight / 2, cardWidth, cardHeight);
	context.fillStyle = `rgb(${bgR},${bgG},${bgB})`;
	context.fill();
	context.strokeStyle = `rgba(${fgR},${fgG},${fgB},0.2)`;
	context.lineWidth = 1;
	context.stroke();

	if (label) {
		context.textAlign = "left";
		context.fillStyle = `rgb(${fgR},${fgG},${fgB})`;
		context.fillText(label, textX, cardY);
	}

	if (!badge) return;
	drawBadge(
		context,
		badge,
		textX + textWidth + (label ? BADGE_GAP : 0),
		cardY,
		fontSize,
		font,
		1,
	);
}
