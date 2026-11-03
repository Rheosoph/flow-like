import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

export const musicShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "tempo",
		label: "Tempo",
		series: "music",
		description: "A wooden metronome keeping time with a golden pendulum.",
		palette: {
			body: "#b77d59",
			background: "#f4e4d6",
			accent: "#e8bc66",
			sclera: "#fff1d6",
			clothing: "#677f83",
		},
		mouthStyle: "line",
		silhouette: "M39 23H61L82 99H18Z",
		anchors: {
			head: { x: 50, y: 25, width: 28 },
			eyes: { left: [40, 73], right: [60, 73], radius: 3.3 },
			mouth: [50, 85],
			neck: { y: 89, bottom: 101 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M42 30H58L72 90H28Z" fill={palette.sclera} />
				<path
					d="M45 34H55M43 43H57M41 52H59"
					stroke={palette.body}
					strokeWidth="1"
					opacity=".35"
				/>
				<BotMotion
					kind="pendulum"
					origin={[50, 59]}
					amount={18}
					duration={1800}
				>
					<path d="M50 59V32" stroke={palette.ink} strokeWidth="2.2" />
					<path d="M45 38H55V47H45Z" fill={palette.accent} />
				</BotMotion>
				<circle cx="50" cy="59" r="3" fill={palette.body} />
				<path d="M23 95H77" stroke={palette.ink} strokeWidth="2" opacity=".3" />
			</>
		),
	}),
	crewShape({
		id: "vinyl",
		label: "Vinyl",
		series: "music",
		description: "A record with a peach label and concentric grooves.",
		palette: {
			body: "#3c3b4e",
			background: "#f2ddd8",
			accent: "#f0af92",
			clothing: "#70718d",
		},
		silhouette: ovalPath(50, 65, 37),
		anchors: {
			head: { x: 50, y: 29, width: 43 },
			eyes: { left: [41, 64], right: [59, 64], radius: 3.2 },
			mouth: [50, 75],
			neck: { y: 84, bottom: 103 },
		},
		renderDetails: ({ palette }) => (
			<>
				<g fill="none" stroke={palette.clothing} strokeWidth="1.2">
					<circle cx="50" cy="65" r="32" />
					<circle cx="50" cy="65" r="28" />
					<circle cx="50" cy="65" r="24" />
				</g>
				<circle cx="50" cy="65" r="20" fill={palette.accent} />
				<circle cx="50" cy="53" r="2.5" fill={palette.body} />
				<BotMotion kind="spin" origin={[50, 65]} duration={14000}>
					<path
						d="M23 44L28 38M18 53L32 36M73 86L68 92"
						stroke={palette.sclera}
						strokeWidth="2"
						opacity=".35"
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "boombox",
		label: "Boombox",
		series: "music",
		description: "A mint radio with twin speakers and a carry handle.",
		palette: {
			body: "#91c6bb",
			background: "#e5efdf",
			accent: "#e9bb65",
			clothing: "#557f7e",
		},
		silhouette:
			"M16 44H84Q90 44 90 50V93Q90 99 84 99H16Q10 99 10 93V50Q10 44 16 44Z",
		anchors: {
			head: { x: 50, y: 36, width: 46 },
			eyes: { left: [42, 67], right: [58, 67], radius: 3 },
			mouth: [50, 78],
			neck: { y: 85, bottom: 101 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<path
						d="M31 44V33Q31 29 35 29H65Q69 29 69 33V44"
						fill="none"
						stroke={palette.clothing}
						strokeWidth="5"
					/>
				)}
				<path
					d="M20 44L13 24"
					stroke={palette.clothing}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				<g fill={palette.clothing}>
					<circle cx="24" cy="75" r="10" />
					<circle cx="76" cy="75" r="10" />
				</g>
				{[24, 76].map((x) => (
					<BotMotion
						key={x}
						kind="pulse"
						origin={[x, 75]}
						amount={12}
						duration={1600}
					>
						<circle cx={x} cy="75" r="5" fill={palette.body} />
					</BotMotion>
				))}
				<rect
					x="32"
					y="50"
					width="36"
					height="7"
					rx="2"
					fill={palette.sclera}
				/>
				<path
					d="M38 51V56M46 51V56M54 51V56M62 51V56"
					stroke={palette.clothing}
					strokeWidth="1"
				/>
				<circle cx="80" cy="54" r="3" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "strum",
		label: "Strum",
		series: "music",
		description: "A honey-colored ukulele with a small round sound hole.",
		palette: {
			body: "#e2aa58",
			background: "#f8e7c7",
			accent: "#9f643d",
			clothing: "#5e8c8a",
			sclera: "#fff0d0",
		},
		cheeks: true,
		silhouette:
			"M43 17H57V48C71 45 78 56 72 66C87 78 80 101 50 103C20 101 13 78 28 66C22 56 29 45 43 48Z",
		anchors: {
			head: { x: 50, y: 20, width: 23 },
			eyes: { left: [39, 75], right: [61, 75], radius: 3.4 },
			mouth: [50, 85],
			neck: { width: 50, y: 90, bottom: 103 },
		},
		renderDetails: ({ palette }) => (
			<>
				<rect x="44" y="29" width="12" height="29" fill={palette.accent} />
				<circle cx="50" cy="62" r="8" fill={palette.accent} />
				<circle cx="50" cy="62" r="5" fill={palette.ink} />
				<path d="M48 22V66M52 22V66" stroke={palette.sclera} strokeWidth=".8" />
				<path
					d="M40 22H44M56 22H60M40 28H44M56 28H60"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<path
					d="M39 97H61"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "clef",
		label: "Clef",
		series: "music",
		description: "A violet musical note with a curled flag.",
		palette: {
			body: "#b99ad9",
			background: "#eee4f7",
			accent: "#e0c5f4",
			clothing: "#705b92",
		},
		silhouette:
			"M65 24Q65 20 69 20H73C73 32 91 33 86 49C83 42 78 41 73 40V74C73 90 59 101 43 101C26 101 17 91 21 77C25 61 45 55 65 63Z",
		anchors: {
			head: { x: 68, y: 22, width: 24 },
			eyes: { left: [38, 70], right: [57, 70], radius: 3.3 },
			mouth: [47, 80],
			neck: { x: 47, width: 44, y: 86, bottom: 100 },
		},
		renderDetails: ({ palette }) => (
			<path
				d="M28 73Q35 63 47 64"
				fill="none"
				stroke={palette.accent}
				strokeWidth="3"
				strokeLinecap="round"
			/>
		),
	}),
	crewShape({
		id: "chime",
		label: "Chime",
		series: "music",
		description: "A brass bell with a round clapper and a bright rim.",
		palette: {
			body: "#e9bb55",
			background: "#f7ecd1",
			accent: "#b78532",
			clothing: "#78988d",
		},
		mouthStyle: "o",
		silhouette:
			"M44 32V27Q50 20 56 27V32C72 35 75 48 75 65L86 85Q88 91 81 91H19Q12 91 14 85L25 65C25 48 28 35 44 32Z",
		anchors: {
			head: { x: 50, y: 29, width: 29 },
			eyes: { left: [39, 62], right: [61, 62], radius: 3.7 },
			mouth: [50, 75],
			neck: { y: 81, bottom: 93 },
		},
		renderDetails: ({ palette }) => (
			<>
				<BotMotion
					kind="pendulum"
					origin={[50, 87]}
					amount={12}
					duration={2400}
				>
					<path
						d="M46 87H54V94Q58 94 58 98Q50 105 42 98Q42 94 46 94Z"
						fill={palette.accent}
					/>
				</BotMotion>
				<path
					d="M19 85H81"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<path
					d="M33 51Q34 41 42 39"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".7"
				/>
			</>
		),
	}),
];
