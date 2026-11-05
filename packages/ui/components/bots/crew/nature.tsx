import { BotPaintBlend } from "../bot-paints";
import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

const dripBody =
	"M50 23C41 38 13 62 13 82C13 107 87 107 87 82C87 62 59 38 50 23Z";

const petals = [
	[50, 40],
	[76, 55],
	[76, 81],
	[50, 94],
	[24, 81],
	[24, 55],
] as const;

export const natureShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "sprout",
		backgroundDetailColors: ["accent"],
		label: "Sprout",
		series: "nature",
		description: "A peach-colored seedling with two green leaves.",
		palette: {
			body: "#ee9488",
			background: "#fce3df",
			cheek: "#e47c73",
			accent: "#5f9c6c",
			clothing: "#576b89",
		},
		cheeks: true,
		silhouette: "M12 82C12 60 28 42 50 42C72 42 88 60 88 82V102H12Z",
		anchors: {
			eyes: { left: [39, 71], right: [61, 71], radius: 3.7 },
			mouth: [50, 83],
			head: { x: 50, y: 42, width: 46 },
			neck: { y: 87, bottom: 105 },
		},
		renderDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<BotMotion kind="sway" origin={[49, 44]} duration={5200} amount={6}>
					<g fill={palette.accent}>
						<path
							d="M49 44Q47 37 48 32"
							stroke={palette.accent}
							strokeWidth="2"
							fill="none"
						/>
						<path d="M49 40C36 41 33 32 32 28C44 28 48 32 49 40Z" />
						<path d="M49 40C47 28 54 20 65 19C66 30 59 38 49 40Z" />
					</g>
				</BotMotion>
			),
	}),
	crewShape({
		id: "drip",
		label: "Drip",
		series: "nature",
		description: "A blue water drop with a quiet, mouthless face.",
		palette: {
			body: "#59a8d6",
			background: "#dceff9",
			accent: "#b7e1ed",
			clothing: "#665887",
		},
		mouthStyle: "none",
		silhouette: dripBody,
		anchors: {
			eyes: { left: [39, 68], right: [61, 68], radius: 3.8 },
			mouth: [50, 79],
			head: { x: 50, y: 31, width: 33 },
			neck: { y: 86, bottom: 100, width: 50 },
		},
		renderDetails: (context) => (
			<>
				<BotPaintBlend
					context={context}
					id="drip"
					d={dripBody}
					from="accent"
					to="body"
					end={0.85}
				/>
				<path
					d="M26 66Q29 57 37 48"
					fill="none"
					stroke={context.palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".65"
				/>
			</>
		),
	}),
	crewShape({
		id: "ember",
		label: "Ember",
		series: "nature",
		description: "An orange flame with a golden glow at its base.",
		palette: {
			body: "#f47749",
			background: "#ffe3d7",
			accent: "#ffcb60",
			clothing: "#645681",
		},
		silhouette:
			"M50 15C32 34 29 49 29 58C23 57 21 51 22 44C3 64 12 92 22 101H78C94 85 90 60 77 42C79 52 75 56 73 58C68 39 58 23 50 15Z",
		anchors: {
			eyes: { left: [39, 66], right: [61, 66], radius: 3.8 },
			mouth: [50, 84],
			head: { x: 50, y: 28, width: 35 },
			neck: { y: 87, bottom: 103 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					className="fl-bot-crew-motion fl-bot-ember-core"
					data-motion-part="ember-core"
					d="M50 72C42 81 36 85 36 94Q36 107 50 107Q65 107 65 94C65 85 57 78 50 72Z"
					fill={palette.accent}
				/>
				<path
					className="fl-bot-crew-motion fl-bot-ember-glow"
					data-motion-part="ember-glow"
					d="M50 84C46 90 42 93 44 99Q50 105 56 99C58 93 54 90 50 84Z"
					fill={palette.sclera}
					opacity=".28"
				/>
				<g fill={palette.accent}>
					{[
						[35, 37],
						[56, 20],
						[69, 42],
					].map(([x, y], index) => (
						<ellipse
							key={`${x}-${y}`}
							className="fl-bot-crew-motion fl-bot-ember-speck"
							data-motion-part="ember-speck"
							cx={x}
							cy={y}
							rx="1.2"
							ry="2"
							opacity="0"
							style={{ animationDelay: `${index * -1.3}s` }}
						/>
					))}
				</g>
			</>
		),
	}),
	crewShape({
		id: "porci",
		backgroundDetailColors: ["accent"],
		label: "Porci",
		series: "nature",
		description: "A little mushroom under a red spotted cap.",
		palette: {
			body: "#f4e8cf",
			background: "#e4ecd6",
			accent: "#e66550",
			cheek: "#eeb69e",
			clothing: "#547b76",
		},
		cheeks: true,
		silhouette:
			"M6 55C6 28 26 16 50 16S94 28 94 55Q94 61 88 61H74V100H26V61H12Q6 61 6 55Z",
		anchors: {
			eyes: { left: [40, 74], right: [60, 74], radius: 3.4 },
			mouth: [50, 85],
			head: { x: 50, y: 23, width: 42 },
			neck: { y: 89, bottom: 104 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M6 55C6 28 26 16 50 16S94 28 94 55Q94 61 88 61H12Q6 61 6 55Z"
					fill={palette.accent}
				/>
				<g fill={palette.sclera}>
					<circle cx="29" cy="37" r="5.2" />
					<circle cx="53" cy="28" r="4.1" />
					<circle cx="72" cy="41" r="5.2" />
					<circle cx="46" cy="48" r="3" />
				</g>
			</>
		),
	}),
	crewShape({
		id: "bloom",
		backgroundDetailColors: ["sclera"],
		label: "Bloom",
		series: "nature",
		description: "A yellow daisy face surrounded by white petals.",
		palette: {
			body: "#f8ce55",
			background: "#dfedd8",
			accent: "#70a16c",
			clothing: "#66769b",
			sclera: "#fffef7",
		},
		silhouette:
			petals.map(([x, y]) => ovalPath(x, y, 15.5)).join(" ") +
			ovalPath(50, 71, 24),
		anchors: {
			eyes: { left: [42, 69], right: [58, 69], radius: 3.1 },
			mouth: [50, 80],
			head: { x: 50, y: 33, width: 35 },
			neck: { y: 86, bottom: 107 },
		},
		renderDetails: ({ palette }) => (
			<>
				<g fill={palette.sclera}>
					{petals.map(([cx, cy]) => (
						<circle key={`${cx}-${cy}`} cx={cx} cy={cy} r="15.5" />
					))}
				</g>
				<circle cx="50" cy="71" r="24" fill={palette.body} />
			</>
		),
	}),
];
