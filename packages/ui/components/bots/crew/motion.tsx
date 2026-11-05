import type { CSSProperties, ReactNode } from "react";

type DetailMotion =
	| "sway"
	| "pendulum"
	| "flutter"
	| "spin"
	| "bob"
	| "pulse"
	| "rise"
	| "flicker";

const defaults: Record<DetailMotion, readonly [number, number]> = {
	sway: [4200, 5],
	pendulum: [1800, 16],
	flutter: [3000, 7],
	spin: [16000, 360],
	bob: [3600, 2],
	pulse: [3200, 6],
	rise: [4200, 9],
	flicker: [2600, 5],
};

/** Animate a movable part around its attachment point in the 100 × 100 rig. */
export function BotMotion({
	kind,
	origin = [50, 50],
	duration = defaults[kind][0],
	delay = 0,
	amount = defaults[kind][1],
	children,
}: {
	kind: DetailMotion;
	origin?: readonly [number, number];
	duration?: number;
	delay?: number;
	amount?: number;
	children: ReactNode;
}) {
	return (
		<g
			className="fl-bot-detail"
			data-detail-motion={kind}
			style={
				{
					transformOrigin: `${origin[0]}px ${origin[1]}px`,
					"--bot-detail-duration": `${duration}ms`,
					"--bot-detail-delay": `${delay}ms`,
					"--bot-detail-angle": `${amount}deg`,
					"--bot-detail-distance": `${amount}px`,
					"--bot-detail-scale": 1 + amount / 100,
				} as CSSProperties
			}
		>
			{children}
		</g>
	);
}
