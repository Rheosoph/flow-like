import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath, radialPath } from "./helpers";
import { BotMotion } from "./motion";

export const cosmosShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "orbi",
		label: "Orbi",
		series: "cosmos",
		description:
			"A little planet with a peach-colored ring and an orbiting moon.",
		palette: {
			body: "#9970c9",
			background: "#ebe2f8",
			accent: "#f1b788",
			clothing: "#493c70",
		},
		silhouette: ovalPath(50, 68, 32),
		anchors: {
			head: { x: 50, y: 36, width: 43 },
			mouth: [50, 70],
			eyes: { left: [40, 60], right: [60, 60], radius: 3.8 },
		},
		renderBackDetails: ({ palette }) => (
			<g className="fl-bot-orbit-plane" data-motion-part="orbit-back">
				<ellipse
					cx="50"
					cy="65"
					rx="40"
					ry="11"
					fill="none"
					stroke={palette.accent}
					strokeWidth="3.6"
				/>
				<circle className="fl-bot-orbit-moon" r="3" fill={palette.sclera} />
			</g>
		),
		renderDetails: ({ palette, idPrefix }) => (
			<>
				<circle cx="33" cy="55" r="6" fill={palette.ink} opacity=".12" />
				<circle cx="65" cy="78" r="4" fill={palette.ink} opacity=".1" />
				<defs>
					<clipPath id={`${idPrefix}-orbit-front`}>
						<path d="M0 65H100V100H0Z" />
					</clipPath>
				</defs>
				<g className="fl-bot-orbit-plane" data-motion-part="orbit-front">
					<path
						d="M10 65a40 11 0 0 0 80 0"
						fill="none"
						stroke={palette.accent}
						strokeWidth="3.6"
					/>
					<g clipPath={`url(#${idPrefix}-orbit-front)`}>
						<circle className="fl-bot-orbit-moon" r="3" fill={palette.sclera} />
					</g>
				</g>
			</>
		),
	}),
	crewShape({
		id: "sol",
		label: "Sol",
		series: "cosmos",
		description: "A sunny face with slowly turning rays.",
		palette: {
			body: "#f28b20",
			background: "#e0eefa",
			accent: "#ffca43",
			clothing: "#395f81",
		},
		silhouette: radialPath(50, 64, [40, 33], 12),
		clothingClip: ovalPath(50, 64, 28),
		bodyMotion: "rotate",
		bodyOrigin: [50, 64],
		anchors: {
			head: { x: 50, y: 29, width: 44 },
			eyes: { left: [40, 62], right: [60, 62], radius: 3.8 },
			mouth: [50, 73],
			neck: { y: 78, bottom: 92, width: 40 },
		},
		renderDetails: ({ palette }) => (
			<circle cx="50" cy="64" r="28" fill={palette.accent} />
		),
	}),
	crewShape({
		id: "stella",
		label: "Stella",
		series: "cosmos",
		description: "A golden star with a few quiet sparkles.",
		palette: {
			body: "#f4c448",
			background: "#e5e8f8",
			accent: "#9aa9dc",
			clothing: "#546ca0",
		},
		silhouette: radialPath(50, 64, [42, 21], 5),
		anchors: {
			head: { x: 50, y: 24, width: 27 },
			eyes: { left: [42, 64], right: [58, 64], radius: 3 },
			mouth: [50, 73],
			neck: { y: 77, bottom: 96 },
		},
		renderDetails: ({ palette }) => (
			<g fill={palette.accent}>
				<path className="fl-bot-star" d="M19 31l2 4 4 2-4 2-2 4-2-4-4-2 4-2Z" />
				<path
					className="fl-bot-star fl-bot-star-1"
					d="M82 78l2 3 3 2-3 2-2 3-2-3-3-2 3-2Z"
				/>
			</g>
		),
	}),
	crewShape({
		id: "roki",
		label: "Roki",
		series: "cosmos",
		description:
			"A hovering rocket with a round porthole and a glowing exhaust.",
		palette: {
			body: "#f0f3f8",
			background: "#d3e1f6",
			accent: "#e9584d",
			clothing: "#4772a1",
			cheek: "#b4d4f3",
			sclera: "#ffe0a4",
		},
		bodyMotion: "float",
		silhouette: "M32 40Q33 26 50 13Q67 26 68 40V81Q50 89 32 81Z",
		anchors: {
			head: { x: 50, y: 16, width: 30 },
			eyes: { left: [43, 53], right: [57, 53], radius: 3 },
			mouth: [50, 65],
			neck: { y: 73, bottom: 85 },
			hand: [29, 80],
		},
		renderBackDetails: ({ palette }) => (
			<BotMotion kind="flicker" origin={[50, 82]} duration={1800} amount={10}>
				<path d="M41 80Q38 90 50 96Q62 90 59 80Z" fill={palette.accent} />
				<path d="M46 81Q44 89 50 93Q56 89 54 81Z" fill={palette.sclera} />
			</BotMotion>
		),
		renderDetails: ({ palette }) => (
			<>
				<path d="M32 36Q39 20 50 13Q61 20 68 36Z" fill={palette.accent} />
				<path
					d="M32 62L20 77V86L32 78ZM68 62L80 77V86L68 78Z"
					fill={palette.accent}
				/>
				<circle
					cx="50"
					cy="56"
					r="16"
					fill={palette.cheek}
					stroke={palette.clothing}
					strokeWidth="1.5"
				/>
				<path
					d="M39 50Q42 44 48 43"
					fill="none"
					stroke={palette.body}
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "astro",
		label: "Astro",
		series: "cosmos",
		description: "A tiny astronaut behind a dark visor.",
		palette: {
			body: "#f3f4fa",
			background: "#e5e2f5",
			ink: "#f3f7ff",
			accent: "#627ba4",
			clothing: "#303e5e",
			cheek: "#c7cbdc",
		},
		silhouette: ovalPath(50, 66, 36, 32),
		mouthColor: "sclera",
		eyeStyle: "light",
		bodyMotion: "float",
		anchors: {
			head: { x: 50, y: 34, width: 53 },
			eyes: { left: [40, 66], right: [60, 66], radius: 3.1 },
			mouth: [50, 76],
			neck: { y: 84, bottom: 98 },
		},
		renderDetails: ({ palette }) => (
			<>
				<ellipse cx="50" cy="66" rx="27" ry="20" fill={palette.clothing} />
				<path
					d="M29 60Q31 51 41 50"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="3"
					opacity=".8"
					strokeLinecap="round"
				/>
				<circle cx="14" cy="66" r="5" fill={palette.cheek} />
				<circle cx="86" cy="66" r="5" fill={palette.cheek} />
			</>
		),
	}),
];
