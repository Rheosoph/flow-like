import type { BotShapeDefinition } from "../types";
import { crewShape } from "./helpers";
import { BotMotion } from "./motion";

export const arcaneShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "brim",
		label: "Brim",
		series: "arcane",
		description:
			"A floppy wizard hat with a crescent buckle and a nodding tip.",
		palette: {
			body: "#856794",
			background: "#eee2ef",
			accent: "#dfbc72",
			sclera: "#fff1d9",
			clothing: "#56476b",
			ink: "#32263d",
		},
		silhouette:
			"M27 79L39 37Q49 26 63 37L77 79Q94 83 92 91Q50 108 8 91Q6 83 27 79Z",
		eyeStyle: "light",
		mouthColor: "sclera",
		closedEyeColor: "sclera",
		anchors: {
			eyes: { left: [41, 62], right: [59, 62], radius: 3.3 },
			mouth: [50, 73],
			head: { x: 50, y: 32, width: 35 },
			neck: { y: 85, bottom: 101 },
		},
		renderBackDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<BotMotion kind="sway" origin={[49, 40]} duration={5400} amount={4}>
					<path
						d="M38 40L48 15Q53 6 67 15L78 28Q65 24 59 32L63 40Z"
						fill={palette.body}
					/>
				</BotMotion>
			),
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M27 79Q50 85 77 79L81 87Q50 95 22 87Z"
					fill={palette.clothing}
				/>
				<path
					d="M54 82A5 5 0 1 0 54 91A4 4 0 0 1 54 82Z"
					fill={palette.accent}
				/>
				<path d="M50 39l2 4 4 1-4 2-2 4-1-4-4-2 4-1Z" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "phial",
		faceColor: "accent",
		label: "Phial",
		series: "arcane",
		description: "A corked potion bottle filled with mint-green bubbles.",
		palette: {
			body: "#b4dcd9",
			background: "#e3ecf1",
			accent: "#5caf9c",
			cheek: "#a07b58",
			clothing: "#4f6878",
			sclera: "#f1fff7",
		},
		silhouette:
			"M39 28H61V44Q82 51 83 75V92Q83 101 74 101H26Q17 101 17 92V75Q18 51 39 44Z",
		anchors: {
			head: { x: 50, y: 28, width: 30 },
			eyes: { left: [39, 70], right: [61, 70], radius: 3.6 },
			mouth: [50, 82],
			neck: { y: 88, bottom: 103 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				<path
					d="M23 61Q38 67 50 61Q64 56 77 61L78 94H22Z"
					fill={palette.accent}
				/>
				{!equipment?.head && (
					<path d="M39 23H61L59 33H41Z" fill={palette.cheek} />
				)}
				<path
					d="M37 35H63"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<g fill={palette.sclera} opacity=".7">
					<BotMotion kind="rise" origin={[56, 52]} duration={4400} amount={7}>
						<circle cx="56" cy="52" r="3" />
					</BotMotion>
					<BotMotion
						kind="rise"
						origin={[44, 54]}
						duration={5200}
						delay={-1800}
						amount={9}
					>
						<circle cx="44" cy="54" r="2" />
					</BotMotion>
					<BotMotion
						kind="rise"
						origin={[29, 81]}
						duration={4600}
						delay={-900}
						amount={9}
					>
						<circle cx="29" cy="81" r="2" />
					</BotMotion>
				</g>
				<path
					d="M28 54Q23 61 23 70"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".7"
				/>
			</>
		),
	}),
	crewShape({
		id: "broom",
		label: "Broom",
		series: "arcane",
		description:
			"A flying broom with a curled handle and swishing golden bristles.",
		palette: {
			body: "#d7b56d",
			background: "#e8e4f0",
			accent: "#9a78ad",
			clothing: "#8b684f",
			sclera: "#f9e6ad",
		},
		silhouette: "M36 53H64L82 96Q50 108 18 96Z",
		anchors: {
			eyes: { left: [40, 73], right: [60, 73], radius: 3.4 },
			mouth: [50, 84],
			head: { x: 50, y: 54, width: 31 },
			neck: { y: 91, bottom: 105 },
		},
		renderBackDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<path
					d="M50 57V26Q50 14 61 17"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="7"
					strokeLinecap="round"
				/>
			),
		renderDetails: ({ palette }) => (
			<>
				<path d="M35 53H65L68 62H32Z" fill={palette.accent} />
				<BotMotion kind="flutter" origin={[50, 59]} duration={4000} amount={2}>
					<path
						d="M32 67L24 94M68 67L76 94M35 88L32 99M65 88L68 99"
						fill="none"
						stroke={palette.sclera}
						strokeWidth="2.5"
						strokeLinecap="round"
					/>
				</BotMotion>
				<path
					d="M44 57H56"
					stroke={palette.sclera}
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "sands",
		label: "Sands",
		series: "arcane",
		description:
			"An hourglass with falling golden sand in a violet wooden frame.",
		palette: {
			body: "#dce4ec",
			background: "#eee8f4",
			accent: "#e9c375",
			clothing: "#8b78a5",
			sclera: "#fffaf0",
		},
		silhouette:
			"M28 31H72Q72 49 55 61Q72 71 72 84H28Q28 71 45 61Q28 49 28 31ZM24 84H76V92H24Z",
		anchors: {
			eyes: { left: [42, 70], right: [58, 70], radius: 3 },
			mouth: [50, 77],
			head: { x: 50, y: 27, width: 46 },
			neck: { width: 40, y: 82, bottom: 92 },
			hand: [27, 78],
		},
		renderBackDetails: ({ palette }) => (
			<path d="M22 30V86M78 30V86" stroke={palette.clothing} strokeWidth="5" />
		),
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M33 38H67Q65 48 50 56Q35 48 33 38ZM50 65Q35 69 31 84H69Q65 69 50 65Z"
					fill={palette.accent}
				/>
				<BotMotion kind="rise" origin={[50, 58]} duration={1700} amount={-8}>
					<circle cx="50" cy="58" r="1" fill={palette.accent} />
				</BotMotion>
				<path
					d="M34 39Q35 49 44 54M33 77Q35 70 41 67"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				<path d="M19 24H81V32H19ZM24 84H76V92H24Z" fill={palette.clothing} />
				<path
					d="M24 27H76M29 87H71"
					stroke={palette.sclera}
					opacity=".35"
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "brew",
		label: "Brew",
		series: "arcane",
		description:
			"A round cauldron with bubbling mint potion and little iron feet.",
		palette: {
			body: "#69627d",
			background: "#e4eee7",
			accent: "#9cd4ac",
			clothing: "#494760",
			sclera: "#edffdc",
			ink: "#302939",
		},
		silhouette: "M25 47H75Q88 61 85 80Q82 98 68 99H32Q18 98 15 80Q12 61 25 47Z",
		eyeStyle: "light",
		mouthColor: "sclera",
		closedEyeColor: "sclera",
		anchors: {
			eyes: { left: [39, 68], right: [61, 68], radius: 3.6 },
			mouth: [50, 81],
			head: { x: 50, y: 44, width: 53 },
			neck: { y: 88, bottom: 101 },
		},
		renderBackDetails: ({ palette }) => (
			<>
				<path
					d="M24 62Q7 53 9 72Q9 83 20 81M76 62Q93 53 91 72Q91 83 80 81"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="5"
				/>
				<path
					d="M28 94L24 104H37L39 96M61 96L63 104H76L72 94"
					fill={palette.clothing}
				/>
			</>
		),
		renderDetails: ({ palette }) => (
			<>
				<ellipse cx="50" cy="47" rx="30" ry="8" fill={palette.clothing} />
				<ellipse cx="50" cy="46" rx="25" ry="4.5" fill={palette.accent} />
				{[
					[40, 32, 4],
					[58, 24, 3],
					[65, 38, 2],
				].map(([x, y, r], i) => (
					<BotMotion
						key={x}
						kind="rise"
						origin={[x, y]}
						duration={3700 + i * 800}
						delay={i * -900}
						amount={7}
					>
						<circle cx={x} cy={y} r={r} fill={palette.accent} />
					</BotMotion>
				))}
				<path
					d="M23 64Q20 74 24 81"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".35"
				/>
			</>
		),
	}),
	crewShape({
		id: "emberwick",
		label: "Emberwick",
		series: "arcane",
		description:
			"A lavender candle with dripping wax and a small golden flame.",
		palette: {
			body: "#b6a3d0",
			background: "#f0e7f3",
			accent: "#f4bc61",
			clothing: "#5d7089",
			sclera: "#e7d8ee",
		},
		silhouette: "M27 40Q50 33 73 40V102H27Z",
		anchors: {
			head: { x: 50, y: 39, width: 43 },
			eyes: { left: [39, 68], right: [61, 68], radius: 3.5 },
			mouth: [50, 81],
			neck: { y: 87, bottom: 104 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<>
						<BotMotion
							kind="flicker"
							origin={[50, 37]}
							duration={2300}
							amount={6}
						>
							<path d="M50 13Q68 34 50 37Q33 34 50 13Z" fill={palette.accent} />
						</BotMotion>
						<path
							d="M50 30V40"
							stroke={palette.ink}
							strokeWidth="2"
							strokeLinecap="round"
						/>
					</>
				)}
				<path
					d="M27 40Q50 33 73 40V47Q69 55 65 47V54Q61 63 57 54V47Q51 55 46 47Q42 53 38 47V57Q33 66 29 57Z"
					fill={palette.sclera}
				/>
				<path
					d="M33 74V94"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".5"
				/>
			</>
		),
	}),
];
