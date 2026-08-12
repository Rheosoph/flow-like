import type { Settings } from "sigma/settings";
import type { NodeDisplayData, PartialButFor } from "sigma/types";
import { getGraphTheme } from "./theme-colors";

type NodeData = PartialButFor<
	NodeDisplayData,
	"x" | "y" | "size" | "label" | "color"
> & {
	/** Population a hub stands for, pre-formatted with its `≥` bound. */
	badge?: string;
};

const BADGE_GAP = 6;
const BADGE_PADDING_X = 5;
const BADGE_HEIGHT = 15;

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

	const x = data.x + data.size + 6;
	const y = data.y;

	context.font = `${weight} ${size}px ${font}`;
	context.textAlign = "left";
	context.textBaseline = "middle";

	const [bgR, bgG, bgB] = theme.bgRgb;
	context.strokeStyle = `rgba(${bgR},${bgG},${bgB},0.85)`;
	context.lineWidth = 3;
	context.lineJoin = "round";
	context.strokeText(data.label, x, y);

	context.fillStyle = `rgb(${fgR},${fgG},${fgB})`;
	context.fillText(data.label, x, y);

	if (!data.badge) return;

	const badgeX = x + context.measureText(data.label).width + BADGE_GAP;
	context.font = `600 ${size - 1}px ${font}`;
	const badgeWidth =
		context.measureText(data.badge).width + BADGE_PADDING_X * 2;

	const top = y - BADGE_HEIGHT / 2;
	const radius = BADGE_HEIGHT / 2;
	context.beginPath();
	// Hand-rolled rather than roundRect: this runs inside the render loop, where
	// an unsupported call would take the whole canvas down rather than one pill.
	context.moveTo(badgeX + radius, top);
	context.lineTo(badgeX + badgeWidth - radius, top);
	context.arcTo(
		badgeX + badgeWidth,
		top,
		badgeX + badgeWidth,
		top + radius,
		radius,
	);
	context.lineTo(badgeX + badgeWidth, top + BADGE_HEIGHT - radius);
	context.arcTo(
		badgeX + badgeWidth,
		top + BADGE_HEIGHT,
		badgeX + badgeWidth - radius,
		top + BADGE_HEIGHT,
		radius,
	);
	context.lineTo(badgeX + radius, top + BADGE_HEIGHT);
	context.arcTo(
		badgeX,
		top + BADGE_HEIGHT,
		badgeX,
		top + BADGE_HEIGHT - radius,
		radius,
	);
	context.lineTo(badgeX, top + radius);
	context.arcTo(badgeX, top, badgeX + radius, top, radius);
	context.closePath();
	context.fillStyle = `rgba(${fgR},${fgG},${fgB},${theme.isDark ? 0.16 : 0.1})`;
	context.fill();

	context.fillStyle = `rgba(${fgR},${fgG},${fgB},0.75)`;
	context.fillText(data.badge, badgeX + BADGE_PADDING_X, y);
}

export function drawNodeHover(
	context: CanvasRenderingContext2D,
	data: NodeData,
	_settings: Settings,
): void {
	const theme = getGraphTheme();
	const [fgR, fgG, fgB] = theme.fgRgb;

	const { x, y, size } = data;

	context.beginPath();
	context.arc(x, y, size + 4, 0, Math.PI * 2);
	context.fillStyle = theme.isDark
		? `rgba(${fgR},${fgG},${fgB},0.08)`
		: "rgba(0,0,0,0.06)";
	context.fill();
}
