import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

export const piratesShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "helm",
		label: "Helm",
		series: "pirates",
		description: "A wooden ship's wheel with brass handles and a gentle turn.",
		palette: {
			body: "#c48a55",
			background: "#deeceb",
			accent: "#eac57b",
			clothing: "#85604c",
			sclera: "#fff0ce",
		},
		silhouette: ovalPath(50, 56, 23),
		anchors: {
			eyes: { left: [40, 52], right: [60, 52], radius: 3.3 },
			mouth: [50, 64],
			head: { x: 50, y: 26, width: 42 },
			neck: { y: 70, bottom: 80 },
			hand: [25, 73],
		},
		renderBackDetails: ({ palette }) => (
			<BotMotion kind="pendulum" origin={[50, 56]} duration={6000} amount={8}>
				<g fill="none" strokeLinecap="round">
					{[0, 45, 90, 135, 180, 225, 270, 315].map((angle) => (
						<g key={angle} transform={`rotate(${angle} 50 56)`}>
							<path d="M50 37V17" stroke={palette.clothing} strokeWidth="5" />
							<path d="M50 21V17" stroke={palette.accent} strokeWidth="7" />
						</g>
					))}
					<circle
						cx="50"
						cy="56"
						r="30"
						stroke={palette.body}
						strokeWidth="7"
					/>
					<circle
						cx="50"
						cy="56"
						r="30"
						stroke={palette.accent}
						strokeWidth="1.5"
					/>
				</g>
			</BotMotion>
		),
		renderDetails: ({ palette }) => (
			<circle
				cx="50"
				cy="56"
				r="22"
				fill="none"
				stroke={palette.accent}
				strokeWidth="2"
			/>
		),
	}),
	crewShape({
		id: "coffer",
		label: "Coffer",
		series: "pirates",
		description: "A round-lidded treasure chest with brass corners.",
		palette: {
			body: "#aa624b",
			background: "#f5e3d8",
			accent: "#e8b557",
			clothing: "#3c6373",
			sclera: "#fff1d2",
		},
		silhouette:
			"M15 55Q15 28 50 28Q85 28 85 55V97Q85 102 79 102H21Q15 102 15 97Z",
		anchors: {
			eyes: { left: [38, 69], right: [62, 69], radius: 3.8 },
			mouth: [50, 81],
			head: { x: 50, y: 29, width: 53 },
			neck: { y: 86, bottom: 104 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M17 54H83M27 34V100M73 34V100"
					fill="none"
					stroke={palette.accent}
					strokeWidth="5"
				/>
				<path
					d="M15 55H85"
					stroke={palette.ink}
					strokeWidth="2"
					opacity=".35"
				/>
				<path d="M44 87H56V99H44Z" fill={palette.accent} />
				<path
					d="M50 90V95"
					stroke={palette.ink}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<circle cx="27" cy="59" r="1.5" fill={palette.sclera} />
				<circle cx="73" cy="59" r="1.5" fill={palette.sclera} />
			</>
		),
	}),
	crewShape({
		id: "spyglass",
		label: "Spyglass",
		series: "pirates",
		description: "A brass telescope with a blue lens and nested sliding tubes.",
		palette: {
			body: "#d5aa58",
			background: "#e6ebdf",
			accent: "#f3d894",
			clothing: "#658fa4",
			sclera: "#eefbff",
		},
		silhouette: "M29 27H71V53H67V80H62V99H38V80H33V53H29Z",
		anchors: {
			eyes: { left: [42, 63], right: [58, 63], radius: 3.1 },
			mouth: [50, 74],
			head: { x: 50, y: 22, width: 40 },
			neck: { y: 82, bottom: 100 },
			hand: [32, 84],
		},
		renderDetails: ({ palette }) => (
			<>
				<ellipse cx="50" cy="27" rx="23" ry="10" fill={palette.accent} />
				<ellipse cx="50" cy="27" rx="17" ry="6.5" fill={palette.clothing} />
				<BotMotion kind="sway" origin={[50, 27]} duration={4800} amount={6}>
					<path
						d="M39 27Q43 23 51 23"
						fill="none"
						stroke={palette.sclera}
						strokeWidth="2"
						strokeLinecap="round"
					/>
				</BotMotion>
				<path
					d="M29 45H71V53H29ZM33 79H67V85H33ZM38 94H62V100H38Z"
					fill={palette.accent}
				/>
				<path
					d="M34 36V43M39 55V74"
					stroke={palette.sclera}
					opacity=".5"
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "cask",
		label: "Cask",
		series: "pirates",
		description: "A sturdy ship's barrel with curved wooden staves.",
		palette: {
			body: "#c28a53",
			background: "#f1e7d8",
			accent: "#507b80",
			clothing: "#5c687d",
			sclera: "#f2d6a3",
		},
		silhouette: "M24 33Q50 24 76 33Q91 65 76 99Q50 108 24 99Q9 65 24 33Z",
		anchors: {
			head: { x: 50, y: 31, width: 48 },
			eyes: { left: [39, 65], right: [61, 65], radius: 3.8 },
			mouth: [50, 77],
			neck: { y: 83, bottom: 103 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M35 32Q27 65 35 101M65 32Q73 65 65 101"
					fill="none"
					stroke={palette.ink}
					strokeWidth="1.6"
					opacity=".2"
				/>
				<path
					d="M22 40Q50 47 78 40L81 49Q50 55 19 49ZM20 87Q50 94 80 87L77 96Q50 103 23 96Z"
					fill={palette.accent}
				/>
				<ellipse
					cx="50"
					cy="34"
					rx="24"
					ry="5"
					fill={palette.sclera}
					opacity=".55"
				/>
			</>
		),
	}),
	crewShape({
		id: "anchor",
		label: "Anchor",
		series: "pirates",
		description: "A steel anchor with curved flukes and a loose rope.",
		palette: {
			body: "#839aaa",
			background: "#e3e9f1",
			accent: "#c2d9df",
			clothing: "#d2aa75",
			sclera: "#f3faf9",
		},
		silhouette:
			"M34 43H66V80Q77 78 80 69L72 69L84 56L94 75L87 73Q84 94 50 103Q16 94 13 73L6 75L16 56L28 69H20Q23 78 34 80Z",
		anchors: {
			eyes: { left: [42, 60], right: [58, 60], radius: 3.3 },
			mouth: [50, 72],
			head: { x: 50, y: 27, width: 33 },
			neck: { y: 82, bottom: 103 },
			hand: [25, 83],
		},
		renderBackDetails: ({ palette }) => (
			<>
				<circle
					cx="50"
					cy="30"
					r="10"
					fill="none"
					stroke={palette.body}
					strokeWidth="6"
				/>
				<BotMotion kind="sway" origin={[50, 21]} duration={5400} amount={4}>
					<path
						d="M50 21C36 15 25 21 23 33Q20 43 12 43"
						fill="none"
						stroke={palette.clothing}
						strokeWidth="3"
						strokeLinecap="round"
					/>
				</BotMotion>
			</>
		),
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M24 42H76"
					stroke={palette.accent}
					strokeWidth="7"
					strokeLinecap="round"
				/>
				<path
					d="M24 79Q32 89 45 93M56 93Q66 90 73 84"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "squawk",
		label: "Squawk",
		series: "pirates",
		description: "A scarlet parrot with teal wings and a golden beak.",
		palette: {
			body: "#d96861",
			background: "#f7e4d3",
			accent: "#eeb85b",
			clothing: "#42958e",
			sclera: "#fff4d8",
		},
		silhouette:
			"M23 67Q18 35 49 31Q80 31 78 66L85 96L65 93L51 106L37 94L16 98Z",
		mouthStyle: "beak",
		anchors: {
			eyes: { left: [39, 59], right: [61, 59], radius: 3.7 },
			mouth: [50, 72],
			head: { x: 50, y: 33, width: 43 },
			neck: { y: 82, bottom: 103 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				<path
					d="M22 68Q13 76 17 98L35 87ZM78 68Q87 76 83 98L65 87Z"
					fill={palette.clothing}
				/>
				{!equipment?.head && (
					<path
						d="M39 34Q33 23 44 25L48 31Q49 18 57 23L55 34Z"
						fill={palette.body}
					/>
				)}
				<path d="M37 45Q50 39 63 45L66 66Q50 76 34 66Z" fill={palette.sclera} />
			</>
		),
	}),
];
