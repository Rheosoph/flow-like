import type { BotAnchors } from "./types";

/** Headwear uses a 44-wide crown; hanging pieces leave the eye corridor clear. */
export function getBotHeadwearFit(anchors: BotAnchors) {
	const scaleX = anchors.head.width / 44;
	const scaleY = Math.min(scaleX, Math.max(0.2, (anchors.head.y - 3) / 25));
	const { left, right, radius } = anchors.eyes;
	return {
		scaleX,
		scaleY,
		transform: `translate(${anchors.head.x} ${anchors.head.y}) scale(${scaleX} ${scaleY})`,
		faceLeft:
			(Math.min(left[0], right[0]) - radius - 3 - anchors.head.x) / scaleX,
		faceRight:
			(Math.max(left[0], right[0]) + radius + 3 - anchors.head.x) / scaleX,
		faceTop:
			(Math.min(left[1], right[1]) - radius - 3 - anchors.head.y) / scaleY,
		earY: ((left[1] + right[1]) / 2 - anchors.head.y) / scaleY,
	};
}

/** Headsets wrap the face, including characters with a narrow or offset crown. */
export function getBotHeadsetFit(anchors: BotAnchors) {
	const { left, right, radius } = anchors.eyes;
	return {
		left: Math.min(
			anchors.head.x - anchors.head.width / 2,
			left[0] - radius - 9,
		),
		right: Math.max(
			anchors.head.x + anchors.head.width / 2,
			right[0] + radius + 9,
		),
		top: anchors.head.y - 9,
		earY: (left[1] + right[1]) / 2,
		micX: anchors.mouth[0] + 12,
		micY: anchors.mouth[1] + 5,
	};
}
