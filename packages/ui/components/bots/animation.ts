import type { BotPose } from "./types";

export const BOT_TIMING = Object.freeze({
	blinkMs: 150,
	blinkIntervalMs: [3600, 5400] as const,
	gazeMs: 135,
	gazeHoldMs: [600, 1300] as const,
	hopMs: 1000,
	reactionMs: 650,
	sleepBreathMs: 4800,
	hopIntervalMs: [6500, 10600] as const,
	emotePopMs: 300,
	emoteHoldMs: 1500,
	emoteIntervalMs: [4800, 7200] as const,
	faceFollow: 0.4,
	gazeMaxX: 3.5,
	gazeMaxY: 3,
	shapeRotationMs: 24000,
	lavaFloatMs: 6000,
	haloFlowMs: 5000,
	haloGoldMs: 4000,
	haloAuroraMs: 9000,
	haloPulseMs: 2400,
	haloOrbitMs: 4000,
	goldSparkleMs: 2800,
	wavesMs: 7000,
	raysMs: 60000,
	starsMs: 3200,
	holoMs: 7000,
});

export interface BotGaze {
	x: number;
	y: number;
}

function normalized(value: number): number {
	return Number.isFinite(value) ? Math.max(-1, Math.min(1, value)) : 0;
}

/** Convert normalized input to the pupil's movement in SVG coordinates. */
export function clampGaze(x: number, y: number): BotGaze {
	return {
		x: normalized(x) * BOT_TIMING.gazeMaxX,
		y: normalized(y) * BOT_TIMING.gazeMaxY,
	};
}

export function getPoseGaze(pose: BotPose, gx = 0, gy = 0): BotGaze {
	switch (pose) {
		case "left":
			return clampGaze(-1, 0);
		case "right":
			return clampGaze(1, 0);
		case "up":
			return clampGaze(0, -1);
		case "down":
			return clampGaze(0, 1);
		case "track":
			return clampGaze(gx, gy);
		default:
			return { x: 0, y: 0 };
	}
}

/** Distance is normalized by 1.1 × tile width, then limited radially. */
export function getPointerGaze(
	point: { x: number; y: number },
	rect: { left: number; top: number; width: number; height: number },
): BotGaze {
	if (!Number.isFinite(rect.width) || rect.width <= 0) return { x: 0, y: 0 };
	const dx = point.x - (rect.left + rect.width / 2);
	const dy = point.y - (rect.top + rect.height / 2);
	const distance = Math.hypot(dx, dy);
	if (!Number.isFinite(distance) || distance === 0) return { x: 0, y: 0 };
	const amount = Math.min(1, distance / (1.1 * rect.width));
	return clampGaze((dx / distance) * amount, (dy / distance) * amount);
}
