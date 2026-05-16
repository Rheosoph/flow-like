import type { Settings } from "sigma/settings";
import type { NodeDisplayData, PartialButFor } from "sigma/types";
import { getGraphTheme } from "./theme-colors";

type NodeData = PartialButFor<
	NodeDisplayData,
	"x" | "y" | "size" | "label" | "color"
>;

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
