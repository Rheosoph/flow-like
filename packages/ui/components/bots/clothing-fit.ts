import type { BotAnchors } from "./types";

/** Garment art uses a 48-wide chest and 24-high torso before fitting the character. */
export function getBotClothingFit(anchors: BotAnchors) {
	const x = anchors.neck.x ?? anchors.head.x;
	const width = Math.max(1, anchors.neck.width ?? anchors.head.width);
	const top = anchors.neck.y;
	const bottom = Math.min(100, anchors.neck.bottom);
	const height = Math.max(1, bottom - top);
	const scaleX = width / 48;
	const scaleY = height / 24;
	return {
		x,
		width,
		top,
		bottom,
		height,
		scaleX,
		scaleY,
		transform: `translate(${x} ${top}) scale(${scaleX} ${scaleY})`,
	};
}
