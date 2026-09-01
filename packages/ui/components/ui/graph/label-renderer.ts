import type { Settings } from "sigma/settings";
import type { NodeDisplayData, PartialButFor } from "sigma/types";
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

/** Shortest a caption is ever cut; below this a truncation hides more than it helps. */
const TRUNCATE_MIN_CHARS = 16;
/** Longest caption drawn even fully zoomed in — the hover card carries the rest. */
const TRUNCATE_MAX_CHARS = 44;
const ELLIPSIS = "…";

/** Vertical clearance the hover card keeps around its text. */
const HOVER_PADDING_Y = 4;
const HOVER_PADDING_X = 6;

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

/**
 * Labels culled by size pop in the moment a node crosses the threshold; a short
 * alpha ramp just above it turns that pop into a fade. Forced and highlighted
 * labels are exempt — they bypass the threshold, so the ramp would blank them.
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
	const radius = height / 2;
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
	if (!data.label) return;

	const theme = getGraphTheme();
	const [fgR, fgG, fgB] = theme.fgRgb;

	const size = settings.labelSize;
	const font = settings.labelFont;
	const weight = settings.labelWeight;
	const alpha = labelAlpha(data, settings);
	const label = truncateLabel(data.label, data.size);

	const x = data.x + data.size + 6;
	const y = data.y;

	context.font = `${weight} ${size}px ${font}`;
	context.textAlign = "left";
	context.textBaseline = "middle";

	const [bgR, bgG, bgB] = theme.bgRgb;
	context.strokeStyle = `rgba(${bgR},${bgG},${bgB},${0.85 * alpha})`;
	context.lineWidth = 3;
	context.lineJoin = "round";
	context.strokeText(label, x, y);

	context.fillStyle = `rgba(${fgR},${fgG},${fgB},${alpha})`;
	context.fillText(label, x, y);

	if (!data.badge) return;
	drawBadge(
		context,
		data.badge,
		x + context.measureText(label).width + BADGE_GAP,
		y,
		size,
		font,
		alpha,
	);
}

/**
 * Hover and selection render on the layer above the labels, so this is where
 * the full, untruncated caption belongs: a card behind it keeps it readable
 * over whatever the canvas has underneath.
 */
export function drawNodeHover(
	context: CanvasRenderingContext2D,
	data: NodeData,
	settings: Settings,
): void {
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

	const textX = x + size + 6;
	const textWidth = context.measureText(data.label).width;
	const cardHeight = fontSize + HOVER_PADDING_Y * 2;

	tracePill(
		context,
		textX - HOVER_PADDING_X,
		y - cardHeight / 2,
		textWidth + HOVER_PADDING_X * 2,
		cardHeight,
	);
	context.fillStyle = `rgba(${bgR},${bgG},${bgB},0.92)`;
	context.fill();
	context.strokeStyle = `rgba(${fgR},${fgG},${fgB},0.16)`;
	context.lineWidth = 1;
	context.stroke();

	context.fillStyle = `rgb(${fgR},${fgG},${fgB})`;
	context.fillText(data.label, textX, y);

	if (!data.badge) return;
	drawBadge(
		context,
		data.badge,
		textX + textWidth + BADGE_GAP + HOVER_PADDING_X,
		y,
		fontSize,
		font,
		1,
	);
}
