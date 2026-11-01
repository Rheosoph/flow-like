import type { BotShapeDefinition } from "../types";
import { crewShape } from "./helpers";
import { BotMotion } from "./motion";

const wrenchBody =
	"M36 19V38L50 46L64 38V19C82 27 82 48 65 57L68 86Q68 103 50 103Q32 103 32 86L35 57C18 48 18 27 36 19Z";

export const workshopShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "wrench",
		label: "Wrench",
		series: "workshop",
		description: "A steel wrench with an open jaw and a round handle.",
		palette: {
			body: "#95adbb",
			background: "#e3edf0",
			accent: "#d5e3e9",
			clothing: "#547c90",
		},
		silhouette: wrenchBody,
		clothingClip: `${wrenchBody}M46 94a4 4 0 1 0 8 0a4 4 0 1 0-8 0Z`,
		anchors: {
			head: { x: 50, y: 25, width: 43 },
			eyes: { left: [42, 70], right: [58, 70], radius: 3 },
			mouth: [50, 81],
			neck: { y: 86, bottom: 103, width: 30 },
			hand: [32, 84],
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M28 32Q25 47 39 53M72 32Q75 47 61 53"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				<circle cx="50" cy="94" r="4" fill={palette.background} />
			</>
		),
	}),
	crewShape({
		id: "mallet",
		label: "Mallet",
		series: "workshop",
		description: "A wooden mallet with a broad head and a wrapped handle.",
		palette: {
			body: "#d9a26b",
			background: "#f5e9d9",
			accent: "#af784c",
			clothing: "#759b98",
			sclera: "#f7d7a8",
		},
		silhouette:
			"M17 33H83Q88 33 88 38V70Q88 75 83 75H59V104H41V75H17Q12 75 12 70V38Q12 33 17 33Z",
		anchors: {
			head: { x: 50, y: 34, width: 52 },
			eyes: { left: [39, 52], right: [61, 52], radius: 3.7 },
			mouth: [50, 65],
			neck: { y: 77, bottom: 104, width: 18 },
			hand: [42, 91],
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M21 34V74M79 34V74" stroke={palette.accent} strokeWidth="4" />
				<path d="M41 81H59V104H41Z" fill={palette.clothing} />
				<path
					d="M42 86L58 91M42 94L58 99"
					stroke={palette.sclera}
					strokeWidth="2"
					opacity=".6"
				/>
			</>
		),
	}),
	crewShape({
		id: "bolt",
		label: "Bolt",
		series: "workshop",
		description: "A chunky hex bolt with a shiny head and threaded stem.",
		palette: {
			body: "#b1becd",
			background: "#e8e9f6",
			accent: "#7c8fa7",
			clothing: "#6d709d",
			sclera: "#e8f0f6",
		},
		mouthStyle: "line",
		silhouette: "M31 29H69L85 48L69 73H62V104H38V73H31L15 48Z",
		anchors: {
			head: { x: 50, y: 30, width: 40 },
			eyes: { left: [39, 49], right: [61, 49], radius: 3.4 },
			mouth: [50, 62],
			neck: { y: 76, bottom: 104, width: 24 },
			hand: [37, 88],
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M31 29L36 35H64L69 29M15 48L23 49L36 68L31 73"
					fill={palette.sclera}
				/>
				<path d="M69 29L85 48L69 73L64 68L77 49Z" fill={palette.accent} />
				<path
					d="M38 79L62 83M38 87L62 91M38 95L62 99"
					stroke={palette.accent}
					strokeWidth="3"
				/>
			</>
		),
	}),
	crewShape({
		id: "brush",
		label: "Brush",
		series: "workshop",
		description: "A paintbrush with a teal paint-dipped fringe.",
		palette: {
			body: "#e5bb86",
			background: "#e4f0ec",
			accent: "#68ada5",
			clothing: "#91aab7",
			sclera: "#e9f0ed",
		},
		cheeks: true,
		silhouette: "M23 54H77L70 78H60L58 103H42L40 78H30Z",
		anchors: {
			head: { x: 50, y: 28, width: 42 },
			eyes: { left: [40, 62], right: [60, 62], radius: 3.3 },
			mouth: [50, 73],
			neck: { y: 82, bottom: 103, width: 16 },
			hand: [37, 87],
		},
		renderDetails: ({ palette }) => (
			<>
				<BotMotion kind="sway" origin={[50, 54]} amount={2} duration={5200}>
					<path d="M27 27H73L77 55H23Z" fill={palette.body} />
					<path
						d="M27 27H73L75 40Q68 45 62 40Q55 47 48 41Q39 47 25 40Z"
						fill={palette.accent}
					/>
					<path
						d="M34 42L33 53M44 45V53M56 44V53M66 43L67 53"
						stroke={palette.sclera}
						strokeWidth="1.7"
					/>
				</BotMotion>
				<path d="M23 54H77L70 78H30Z" fill={palette.clothing} />
				<path d="M28 56H72" stroke={palette.sclera} strokeWidth="2" />
				<circle cx="50" cy="96" r="3" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "spool",
		label: "Spool",
		series: "workshop",
		description: "A spool of plum-colored thread with a loose curling end.",
		palette: {
			body: "#b78fbd",
			background: "#f0e6ef",
			accent: "#8b6394",
			clothing: "#719c9f",
			sclera: "#ebd1aa",
		},
		silhouette:
			"M22 35Q50 25 78 35V45L70 49V88L80 92V101H20V92L30 88V49L22 45Z",
		anchors: {
			head: { x: 50, y: 33, width: 48 },
			eyes: { left: [40, 64], right: [60, 64], radius: 3.4 },
			mouth: [50, 77],
			neck: { y: 85, bottom: 103 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M22 35Q50 25 78 35V45Q50 54 22 45ZM30 88Q50 94 70 88L80 92V101H20V92Z"
					fill={palette.sclera}
				/>
				<ellipse cx="50" cy="37" rx="9" ry="3" fill={palette.accent} />
				<path
					d="M30 54H70M30 58H35M65 58H70M30 66H34M66 66H70M30 74H34M66 74H70M30 85H70"
					fill="none"
					stroke={palette.accent}
					strokeWidth="1.5"
					strokeLinecap="round"
				/>
				<BotMotion kind="sway" origin={[31, 81]} amount={7} duration={4400}>
					<path
						d="M31 81Q21 82 19 76Q15 65 10 72"
						fill="none"
						stroke={palette.accent}
						strokeWidth="2"
						strokeLinecap="round"
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "measure",
		label: "Measure",
		series: "workshop",
		description:
			"A pocket tape measure with a little strip of tape pulled out.",
		palette: {
			body: "#edba52",
			background: "#f6eddb",
			accent: "#647184",
			clothing: "#91816a",
			sclera: "#fff1c4",
		},
		silhouette: "M18 60Q18 32 44 32H57Q77 32 77 54V82H93V95H31Q18 95 18 82Z",
		anchors: {
			head: { x: 48, y: 33, width: 42 },
			eyes: { left: [37, 61], right: [58, 61], radius: 3.5 },
			mouth: [47, 74],
			neck: { y: 81, bottom: 97 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M31 38Q22 41 22 60V81Q22 90 31 90H74"
					fill="none"
					stroke={palette.accent}
					strokeWidth="4"
					strokeLinecap="round"
				/>
				<path d="M54 32H65V42H54Z" fill={palette.accent} />
				<path d="M77 83H93V94H77Z" fill={palette.sclera} />
				<path
					d="M82 84V89M87 84V91M93 81V96"
					stroke={palette.accent}
					strokeWidth="2"
				/>
			</>
		),
	}),
];
