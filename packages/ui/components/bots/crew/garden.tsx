import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

export const gardenShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "prickles",
		label: "Prickles",
		series: "garden",
		description: "A cactus with raised arms and a pink flower.",
		palette: {
			body: "#82b686",
			background: "#f4ead8",
			accent: "#e8a4b5",
			sclera: "#c9dfb2",
			clothing: "#b58a68",
		},
		anchors: {
			head: { x: 50, y: 31, width: 32 },
			eyes: { left: [41, 60], right: [59, 60], radius: 3.4 },
			mouth: [50, 73],
			neck: { y: 82, bottom: 103 },
		},
		silhouette:
			"M32 103V78H24Q10 78 10 64V53Q10 43 20 43Q28 43 28 53V61H32V44Q32 27 50 27Q68 27 68 44V61H72V45Q72 35 80 35Q90 35 90 45V64Q90 78 76 78H68V103Z",
		renderDetails: ({ palette, equipment }) => (
			<>
				<path
					d="M38 41V50M62 42V50M36 84V96M64 84V96M17 52V62M82 45V61"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="1.8"
					strokeLinecap="round"
				/>
				{!equipment?.head && (
					<g fill={palette.accent}>
						<circle cx="54" cy="26" r="5" />
						<circle cx="62" cy="26" r="5" />
						<circle cx="58" cy="19" r="5" />
						<circle cx="58" cy="25" r="3" fill={palette.sclera} />
					</g>
				)}
			</>
		),
	}),
	crewShape({
		id: "bonsai",
		label: "Bonsai",
		series: "garden",
		description: "A rounded bonsai tree growing from a blue ceramic pot.",
		palette: {
			body: "#83a878",
			background: "#e7ecdf",
			accent: "#afca94",
			clothing: "#728da8",
			sclera: "#c5d9e4",
		},
		anchors: {
			head: { x: 50, y: 28, width: 44 },
			eyes: { left: [40, 48], right: [60, 48], radius: 3.5 },
			mouth: [50, 59],
			neck: { y: 80, bottom: 99, width: 42 },
		},
		silhouette:
			"M28 65C17 65 12 56 18 49C12 39 21 32 31 34C32 23 44 21 50 29C61 20 74 27 73 36C84 34 91 45 82 53C87 62 75 68 64 65H55V77H77L72 99H28L23 77H45V65Z",
		clothingClip: "M24 81H76L72 99H28Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M46 63H55Q52 70 56 78H44Q49 70 46 63Z"
					fill={palette.ink}
					opacity=".5"
				/>
				<path d="M23 77H77L72 99H28Z" fill={palette.clothing} />
				<path d="M23 77H77V82H23Z" fill={palette.sclera} />
				<path
					d="M28 84L31 96"
					stroke={palette.sclera}
					strokeWidth="2"
					opacity=".45"
					strokeLinecap="round"
				/>
				<path
					d="M23 44Q24 37 33 39M39 31Q44 28 48 33M70 41Q78 38 81 46"
					fill="none"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "fern",
		label: "Fern",
		series: "garden",
		description: "A terracotta plant pot with arching fern leaves.",
		palette: {
			body: "#cf8d74",
			background: "#e5eddf",
			accent: "#6c9c77",
			clothing: "#8a604f",
			sclera: "#edb39b",
		},
		anchors: {
			head: { x: 50, y: 53, width: 47 },
			eyes: { left: [40, 71], right: [60, 71], radius: 3.4 },
			mouth: [50, 83],
			neck: { y: 89, bottom: 103 },
		},
		silhouette: "M21 54H79V63H75L70 103H30L25 63H21Z",
		renderDetails: ({ palette }) => (
			<>
				<BotMotion kind="sway" origin={[50, 54]} duration={5600} amount={4}>
					<g
						fill="none"
						stroke={palette.accent}
						strokeWidth="2.2"
						strokeLinecap="round"
					>
						<path d="M50 54Q49 31 51 14M46 54Q39 31 19 26M54 54Q63 30 84 28" />
					</g>
					<g fill={palette.accent}>
						<path d="M50 26Q38 23 41 15Q49 16 50 26ZM50 36Q62 31 59 23Q51 26 50 36ZM50 46Q36 42 37 33Q48 35 50 46ZM34 36Q23 40 19 32Q27 28 34 36ZM43 46Q29 50 27 41Q36 37 43 46ZM66 40Q66 27 76 24Q79 33 66 40ZM59 50Q70 40 78 46Q75 55 59 50Z" />
					</g>
				</BotMotion>
				<path d="M21 54H79V61H21Z" fill={palette.sclera} />
			</>
		),
	}),
	crewShape({
		id: "acorn",
		label: "Acorn",
		series: "garden",
		description: "A honey-brown acorn wearing a textured woodland cap.",
		palette: {
			body: "#d7ac72",
			background: "#e9eedc",
			accent: "#8d7253",
			clothing: "#698775",
			sclera: "#c2a17a",
		},
		cheeks: true,
		anchors: {
			head: { x: 50, y: 32, width: 44 },
			eyes: { left: [39, 66], right: [61, 66], radius: 3.6 },
			mouth: [50, 79],
			neck: { y: 85, bottom: 104 },
		},
		silhouette:
			"M20 55Q17 33 50 31Q83 33 80 55L77 83Q74 96 50 105Q26 96 23 83Z",
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<path
						d="M50 33Q47 21 56 17"
						fill="none"
						stroke={palette.accent}
						strokeWidth="5"
						strokeLinecap="round"
					/>
				)}
				<path
					d="M17 51Q19 30 50 30Q81 30 83 51Q84 57 77 57H23Q16 57 17 51Z"
					fill={palette.accent}
				/>
				<path
					d="M29 41l5 5 5-5M45 38l5 5 5-5M61 41l5 5 5-5M36 50l5 4 5-4M54 50l5 4 5-4"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="1.5"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "clover",
		label: "Clover",
		series: "garden",
		description:
			"A four-leaf clover with heart-shaped leaves and a curved stem.",
		palette: {
			body: "#83b990",
			background: "#e3f0df",
			accent: "#b9d7a3",
			clothing: "#598a6c",
		},
		anchors: {
			head: { x: 50, y: 31, width: 42 },
			eyes: { left: [41, 58], right: [59, 58], radius: 3.4 },
			mouth: [50, 70],
			neck: { y: 79, bottom: 102 },
		},
		silhouette:
			"M50 43C36 20 14 28 18 43C2 44 11 66 32 65C11 77 18 96 33 92C36 107 55 94 50 78C64 101 84 91 80 77C97 73 88 56 68 60C91 47 81 28 67 33C62 18 45 28 50 43Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M51 84Q58 95 47 105"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="4"
					strokeLinecap="round"
				/>
				<g
					fill="none"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
				>
					<path d="M35 46L27 39M64 45L71 38M31 77L37 73M64 77L71 83" />
				</g>
			</>
		),
	}),
	crewShape({
		id: "berry",
		label: "Berry",
		series: "garden",
		description: "A red strawberry with a leafy crown and golden seeds.",
		palette: {
			body: "#df7983",
			background: "#f5e5df",
			accent: "#80a773",
			sclera: "#f7d8a1",
			clothing: "#6f8a67",
		},
		anchors: {
			head: { x: 50, y: 33, width: 44 },
			eyes: { left: [39, 62], right: [61, 62], radius: 3.6 },
			mouth: [50, 75],
			neck: { y: 82, bottom: 103 },
		},
		silhouette:
			"M15 56C15 35 34 30 50 38C66 30 85 35 85 56C85 77 64 100 50 105C36 100 15 77 15 56Z",
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<path
						d="M50 39L33 26L35 39L20 38L35 48L50 41L65 48L80 38L65 39L67 26ZM48 36Q48 23 54 20L57 23Q51 27 53 37Z"
						fill={palette.accent}
					/>
				)}
				<g fill={palette.sclera}>
					{[
						[26, 54],
						[74, 54],
						[26, 69],
						[74, 69],
						[33, 83],
						[67, 83],
						[45, 93],
						[56, 91],
						[50, 51],
					].map(([x, y]) => (
						<path key={`${x}-${y}`} d={ovalPath(x, y, 1.3, 2.1)} />
					))}
				</g>
			</>
		),
	}),
];
