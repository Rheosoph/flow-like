import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

export const zooShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "kero",
		label: "Kero",
		series: "zoo",
		description: "Frog with raised eyes and a wide smile.",
		palette: {
			body: "#7cbd53",
			background: "#e5f1d9",
			cheek: "#d5b4b4",
			ink: "#2b3432",
			sclera: "#fffdf7",
			clothing: "#517889",
			accent: "#b8d887",
		},
		anchors: {
			eyes: { left: [34, 48], right: [66, 48], radius: 8.7 },
			mouth: [50, 76],
			head: { x: 50, y: 35, width: 49 },
		},
		eyeStyle: "round",
		mouthScale: 1.8,
		silhouette: `${ovalPath(34, 48, 14.5)}${ovalPath(66, 48, 14.5)}${ovalPath(50, 81, 40, 28)}`,
		renderFaceDetails: ({ palette }) => (
			<g fill={palette.cheek}>
				<ellipse cx="25" cy="77" rx="4.5" ry="2.4" />
				<ellipse cx="75" cy="77" rx="4.5" ry="2.4" />
			</g>
		),
	}),
	crewShape({
		id: "axi",
		label: "Axi",
		series: "zoo",
		description: "Pink axolotl with feathery gills.",
		palette: {
			body: "#efa1c0",
			background: "#fbe4ee",
			accent: "#d9679b",
			cheek: "#e77fab",
			ink: "#4a3543",
			clothing: "#7a6598",
		},
		anchors: {
			eyes: { left: [36, 66], right: [64, 66], radius: 3.5 },
			head: { x: 50, y: 42, width: 51 },
			mouth: [50, 78],
		},
		cheeks: true,
		silhouette: "M14 71C14 49 29 40 50 40S86 49 86 71V104H14Z",
		renderDetails: ({ palette }) => (
			<g fill={palette.accent}>
				<BotMotion kind="flutter" origin={[21, 65]} duration={3400} amount={4}>
					<path d="M19 58C7 51 2 37 7 35C14 32 22 47 23 55Z" />
					<path d="M17 64C6 62 -1 54 2 50C6 46 17 55 21 60Z" />
					<path d="M16 70C6 73 -1 68 1 64C3 60 14 63 19 65Z" />
				</BotMotion>
				<BotMotion
					kind="flutter"
					origin={[79, 65]}
					duration={3400}
					delay={-1700}
					amount={4}
				>
					<path d="M81 58C93 51 98 37 93 35C86 32 78 47 77 55Z" />
					<path d="M83 64C94 62 101 54 98 50C94 46 83 55 79 60Z" />
					<path d="M84 70C94 73 101 68 99 64C97 60 86 63 81 65Z" />
				</BotMotion>
			</g>
		),
	}),
	crewShape({
		id: "pino",
		label: "Pino",
		series: "zoo",
		description: "Penguin with a white heart-shaped face.",
		palette: {
			body: "#414f7e",
			background: "#dce4f1",
			sclera: "#fffdf7",
			ink: "#30303c",
			accent: "#eab15e",
			cheek: "#eeb7b6",
			clothing: "#a57f62",
		},
		anchors: {
			eyes: { left: [39, 60], right: [61, 60], radius: 3.8 },
			mouth: [50, 74],
			head: { x: 50, y: 31, width: 47 },
		},
		mouthStyle: "beak",
		cheeks: true,
		silhouette: "M14 70C14 45 26 31 50 31S86 45 86 70V99Q50 115 14 99Z",
		renderDetails: ({ palette }) => (
			<path
				d="M50 52C43 40 25 45 23 61C20 80 35 94 50 99C65 94 80 80 77 61C75 45 57 40 50 52Z"
				fill={palette.sclera}
			/>
		),
	}),
	crewShape({
		id: "piyo",
		label: "Piyo",
		series: "zoo",
		description: "Yellow chick peeking out from an eggshell.",
		palette: {
			body: "#f8d75e",
			background: "#d9eaf7",
			sclera: "#fffdf4",
			ink: "#443b36",
			accent: "#e8a453",
			cheek: "#efb16b",
			clothing: "#cec8b6",
		},
		anchors: { head: { x: 50, y: 28, width: 56 }, mouth: [50, 77] },
		mouthStyle: "beak",
		cheeks: true,
		silhouette:
			"M13 74C13 45 28 28 50 28S87 45 87 74C87 96 70 108 50 108S13 96 13 74Z",
		renderDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<path
					className="fl-bot-crew-motion fl-bot-piyo-shell"
					data-motion-part="piyo-shell"
					d="M15 53C17 34 31 27 50 27S83 34 85 53L77 48L71 55L64 48L57 55L50 48L43 55L36 48L29 55L22 48Z"
					fill={palette.sclera}
					stroke={palette.clothing}
					strokeWidth="1.2"
					strokeLinejoin="round"
				/>
			),
	}),
	crewShape({
		id: "bambu",
		closedEyeColor: "sclera",
		label: "Bambu",
		series: "zoo",
		description: "Panda with round ears and dark eye patches.",
		palette: {
			body: "#f6f7f0",
			background: "#dcecd7",
			ink: "#34333b",
			sclera: "#fffefa",
			cheek: "#d2b4bd",
			clothing: "#67866b",
			accent: "#a2bc85",
		},
		anchors: {
			eyes: { left: [39, 65], right: [61, 65], radius: 6.3 },
			mouth: [50, 83],
			head: { x: 50, y: 40, width: 48 },
			neck: { y: 88, bottom: 100 },
		},
		eyeStyle: "round",
		mouthStyle: "cat",
		silhouette: `${ovalPath(24, 43, 10)}${ovalPath(76, 43, 10)}${ovalPath(50, 76, 38, 36)}`,
		renderDetails: ({ palette }) => (
			<>
				<g fill={palette.ink}>
					<circle cx="24" cy="43" r="10" />
					<circle cx="76" cy="43" r="10" />
				</g>
				<path d={ovalPath(50, 76, 38, 36)} fill={palette.body} />
				<g fill={palette.ink}>
					<ellipse
						cx="39"
						cy="65"
						rx="9.2"
						ry="11.8"
						transform="rotate(22 39 65)"
					/>
					<ellipse
						cx="61"
						cy="65"
						rx="9.2"
						ry="11.8"
						transform="rotate(-22 61 65)"
					/>
				</g>
			</>
		),
		renderFaceDetails: ({ palette }) => (
			<path d="M46.5 77Q50 75 53.5 77L50 81Z" fill={palette.ink} />
		),
	}),
];
