import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

export const arcadeShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "pixel",
		label: "Pixel",
		series: "arcade",
		description: "A lavender handheld console with a mint screen.",
		palette: {
			body: "#b6a5d4",
			background: "#ece6f6",
			accent: "#e995aa",
			sclera: "#dce9c7",
			clothing: "#746986",
		},
		mouthStyle: "line",
		silhouette:
			"M28 27H72Q79 27 79 34V88Q79 104 62 104H28Q21 104 21 97V34Q21 27 28 27Z",
		anchors: {
			head: { x: 50, y: 29, width: 42 },
			eyes: { left: [40, 58], right: [60, 58], radius: 3.4 },
			mouth: [50, 68],
			neck: { y: 80, bottom: 106 },
		},
		renderDetails: ({ palette }) => (
			<>
				<rect
					x="27"
					y="35"
					width="46"
					height="44"
					rx="4"
					fill={palette.clothing}
				/>
				<rect
					x="31"
					y="39"
					width="38"
					height="38"
					rx="2"
					fill={palette.sclera}
				/>
				<path
					d="M32 84H37V89H42V94H37V99H32V94H27V89H32Z"
					fill={palette.clothing}
				/>
				<circle cx="61" cy="94" r="4" fill={palette.accent} />
				<circle cx="71" cy="87" r="4" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "joystick",
		label: "Joystick",
		series: "arcade",
		description: "A teal arcade controller with a round red stick.",
		palette: {
			body: "#76afae",
			background: "#e0efed",
			accent: "#e77873",
			clothing: "#4e757e",
		},
		silhouette:
			"M27 57H73Q80 57 82 64L88 81Q90 86 84 89L75 94Q50 102 25 94L16 89Q10 86 12 81L18 64Q20 57 27 57Z",
		anchors: {
			head: { x: 50, y: 27, width: 30 },
			eyes: { left: [38, 69], right: [62, 69], radius: 3.5 },
			mouth: [50, 79],
			neck: { width: 56, y: 85, bottom: 98 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				<path
					d="M27 58H73Q79 58 81 64Q50 69 19 64Q21 58 27 58Z"
					fill={palette.sclera}
					opacity=".2"
				/>
				<path
					d="M16 87Q50 97 84 87L75 94Q50 102 25 94Z"
					fill={palette.clothing}
					opacity=".22"
				/>
				<ellipse cx="50" cy="58" rx="9" ry="3.5" fill={palette.clothing} />
				<BotMotion
					kind="sway"
					origin={[50, 58]}
					amount={equipment?.head ? 0 : 5}
					duration={3800}
				>
					<path d="M47 44H53V58H47Z" fill={palette.clothing} />
					<circle cx="50" cy="38" r="14" fill={palette.accent} />
					<path
						d="M42 33Q44 28 49 28"
						fill="none"
						stroke={palette.sclera}
						strokeWidth="3"
						strokeLinecap="round"
						opacity=".6"
					/>
				</BotMotion>
				<ellipse cx="72" cy="61" rx="5" ry="2.5" fill={palette.clothing} />
				<ellipse cx="72" cy="60" rx="5" ry="2.5" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "dicey",
		label: "Dicey",
		series: "arcade",
		description: "An ivory game die with rounded corners and coral pips.",
		palette: {
			body: "#fff1d9",
			background: "#f2dfd9",
			accent: "#d8796c",
			clothing: "#8d9bae",
		},
		cheeks: true,
		silhouette:
			"M28 31H72Q86 31 86 45V86Q86 100 72 100H28Q14 100 14 86V45Q14 31 28 31Z",
		anchors: {
			head: { x: 50, y: 32, width: 48 },
			eyes: { left: [39, 65], right: [61, 65], radius: 3.8 },
			mouth: [50, 78],
			neck: { y: 85, bottom: 102 },
		},
		renderDetails: ({ palette }) => (
			<g fill={palette.accent}>
				<circle cx="28" cy="45" r="5" />
				<circle cx="72" cy="45" r="5" />
				<circle cx="28" cy="87" r="5" />
				<circle cx="72" cy="87" r="5" />
			</g>
		),
	}),
	crewShape({
		id: "puzzlo",
		label: "Puzzlo",
		series: "arcade",
		description: "A mint jigsaw piece looking for its next connection.",
		palette: {
			body: "#8abca0",
			background: "#e5eee0",
			accent: "#c5dfaa",
			clothing: "#638b89",
		},
		silhouette:
			"M21 40H41C32 23 68 23 59 40H79V59C96 50 96 82 79 73V101H57C66 84 34 84 43 101H21V76C38 85 38 51 21 60Z",
		anchors: {
			head: { x: 50, y: 30, width: 33 },
			eyes: { left: [41, 63], right: [61, 63], radius: 3.4 },
			mouth: [51, 76],
			neck: { y: 82, bottom: 103 },
		},
		renderDetails: ({ palette }) => (
			<path
				d="M29 46H43M49 33H54"
				stroke={palette.accent}
				strokeWidth="3"
				strokeLinecap="round"
			/>
		),
	}),
	crewShape({
		id: "marble",
		label: "Marble",
		series: "arcade",
		description: "A blue glass marble with ribbons of pink and cream.",
		palette: {
			body: "#96c7df",
			background: "#e4eafa",
			accent: "#dda4c5",
			clothing: "#768fba",
			sclera: "#fff3d4",
		},
		silhouette: ovalPath(50, 65, 35),
		anchors: {
			head: { x: 50, y: 31, width: 41 },
			eyes: { left: [40, 65], right: [60, 65], radius: 3.7 },
			mouth: [50, 77],
			neck: { y: 84, bottom: 102 },
		},
		renderDetails: ({ palette }) => (
			<>
				<BotMotion kind="spin" origin={[50, 65]} duration={24000}>
					<path
						d="M29 37C63 24 85 49 72 83C88 58 80 36 56 31C46 29 36 31 29 37Z"
						fill={palette.accent}
					/>
					<path
						d="M16 60C15 86 40 108 68 94C45 101 24 86 25 65C25 51 40 40 54 42C34 34 19 44 16 60Z"
						fill={palette.sclera}
					/>
				</BotMotion>
				<ellipse
					cx="37"
					cy="42"
					rx="8"
					ry="3"
					transform="rotate(-28 37 42)"
					fill={palette.sclera}
					opacity=".8"
				/>
			</>
		),
	}),
	crewShape({
		id: "domino",
		label: "Domino",
		series: "arcade",
		description: "A peach domino tile with a trio of blue pips.",
		palette: {
			body: "#f1c1a0",
			background: "#f5e8dc",
			accent: "#6689a3",
			clothing: "#b58169",
		},
		silhouette:
			"M32 25H68Q76 25 76 33V96Q76 104 68 104H32Q24 104 24 96V33Q24 25 32 25Z",
		anchors: {
			head: { x: 50, y: 27, width: 38 },
			eyes: { left: [39, 70], right: [61, 70], radius: 3.5 },
			mouth: [50, 82],
			neck: { y: 88, bottom: 104 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M29 58H71" stroke={palette.clothing} strokeWidth="2" />
				<g fill={palette.accent}>
					<circle cx="36" cy="36" r="4" />
					<circle cx="50" cy="43" r="4" />
					<circle cx="64" cy="50" r="4" />
				</g>
			</>
		),
	}),
];
