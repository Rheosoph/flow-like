import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

const inkyHead =
	"M20 58C20 38 31 25 50 25S80 38 80 58V65Q80 74 50 74Q20 74 20 65Z";
const inkyArms = [
	[23, "M23 65Q20 83 14 78"],
	[30, "M30 68Q22 89 28 87"],
	[38, "M38 69Q30 94 37 92"],
	[46, "M46 70Q41 95 46 94"],
	[54, "M54 70Q59 95 54 94"],
	[62, "M62 69Q70 94 63 92"],
	[70, "M70 68Q78 89 72 87"],
	[77, "M77 65Q80 83 86 78"],
] as const;

export const oceanShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "inky",
		label: "Inky",
		series: "ocean",
		description: "A lavender octopus with curled arms and rosy cheeks.",
		palette: {
			body: "#ae86d1",
			background: "#e7eef9",
			accent: "#d4b9e8",
			cheek: "#e6a7c6",
			clothing: "#5d739d",
		},
		cheeks: true,
		anchors: {
			head: { x: 50, y: 26, width: 44 },
			eyes: { left: [39, 49], right: [61, 49], radius: 3.8 },
			mouth: [50, 61],
			neck: { y: 66, bottom: 74 },
		},
		silhouette: inkyHead,
		renderDetails: ({ palette }) => (
			<>
				{inkyArms.map(([x, d], index) => (
					<BotMotion
						key={x}
						kind="flutter"
						origin={[x, 68]}
						duration={3200}
						delay={index * -400}
						amount={5}
					>
						<path
							d={d}
							fill="none"
							stroke={palette.body}
							strokeWidth="7.5"
							strokeLinecap="round"
						/>
						<path
							d={d}
							fill="none"
							stroke={palette.accent}
							strokeWidth="1.5"
							strokeLinecap="round"
							strokeDasharray="1 7"
						/>
					</BotMotion>
				))}
				<path d={inkyHead} fill={palette.body} />
				<ellipse
					cx="34"
					cy="38"
					rx="7"
					ry="3"
					transform="rotate(-30 34 38)"
					fill={palette.accent}
				/>
			</>
		),
	}),
	crewShape({
		id: "shelly",
		faceColor: "accent",
		label: "Shelly",
		series: "ocean",
		description: "A sea turtle peeking out of a moss-green shell.",
		palette: {
			body: "#648c68",
			background: "#deeee4",
			accent: "#acd195",
			clothing: "#526b84",
			cheek: "#d9bd96",
		},
		anchors: {
			head: { x: 50, y: 40, width: 38 },
			eyes: { left: [41, 62], right: [59, 62], radius: 3.5 },
			mouth: [50, 74],
			neck: { y: 84, bottom: 105 },
		},
		silhouette:
			"M22 66Q11 56 8 64Q4 77 17 84L11 101Q24 107 31 96Q50 108 69 96Q76 107 89 101L83 84Q96 77 92 64Q89 56 78 66C79 40 65 30 50 30S21 40 22 66Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M29 87L22 96M71 87L78 96M23 52L33 44M77 52L67 44"
					stroke={palette.accent}
					strokeWidth="2"
					fill="none"
				/>
				<path
					d="M26 65Q26 43 50 43Q74 43 74 65Q74 86 50 86Q26 86 26 65Z"
					fill={palette.accent}
				/>
				<g fill={palette.body} opacity=".45">
					<circle cx="47" cy="48" r="1.5" />
					<circle cx="54" cy="50" r="1.5" />
				</g>
			</>
		),
	}),
	crewShape({
		id: "bubbles",
		label: "Bubbles",
		series: "ocean",
		description: "A round orange fish with a fan tail and little bubbles.",
		palette: {
			body: "#f4a050",
			background: "#daedf6",
			accent: "#e67849",
			sclera: "#fff0d6",
			clothing: "#5b91a0",
		},
		mouthStyle: "o",
		anchors: {
			head: { x: 55, y: 39, width: 43 },
			eyes: { left: [47, 62], right: [67, 62], radius: 3.8 },
			mouth: [57, 74],
			neck: { y: 81, bottom: 98 },
		},
		silhouette: ovalPath(55, 68, 33, 28),
		renderDetails: ({ palette }) => (
			<>
				<BotMotion kind="flutter" origin={[29, 68]} duration={2200} amount={6}>
					<path d="M29 58L9 44Q3 66 9 87L29 76Z" fill={palette.body} />
					<path
						d="M13 53L24 64M12 77L24 70"
						fill="none"
						stroke={palette.accent}
						strokeWidth="2"
					/>
				</BotMotion>
				<path d={ovalPath(55, 68, 33, 28)} fill={palette.body} />
				<BotMotion kind="flutter" origin={[55, 42]} duration={2600} amount={3}>
					<path d="M39 42Q48 27 66 39L65 42" fill={palette.accent} />
				</BotMotion>
				<path d="M45 92Q54 105 69 94L66 91" fill={palette.accent} />
				<path
					d="M30 57Q25 68 31 80"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<g fill="none" stroke={palette.sclera} strokeWidth="2">
					<BotMotion kind="rise" origin={[80, 31]} duration={4200}>
						<circle cx="80" cy="31" r="4" />
					</BotMotion>
					<BotMotion
						kind="rise"
						origin={[70, 20]}
						duration={4200}
						delay={-2100}
					>
						<circle cx="70" cy="20" r="2.5" />
					</BotMotion>
				</g>
			</>
		),
	}),
	crewShape({
		id: "cora",
		label: "Cora",
		series: "ocean",
		description: "A coral friend with branching pink arms.",
		palette: {
			body: "#e58c9c",
			background: "#e0f0ec",
			accent: "#f5c3b6",
			clothing: "#6d81a0",
		},
		cheeks: true,
		anchors: {
			head: { x: 50, y: 48, width: 34 },
			eyes: { left: [41, 69], right: [59, 69], radius: 3.5 },
			mouth: [50, 81],
			neck: { y: 86, bottom: 104 },
		},
		silhouette:
			"M31 103V68Q12 68 12 51V42Q12 34 19 38V50Q19 57 29 57V36L23 31Q19 24 26 23L32 29V22Q32 15 39 20V45L44 48V34Q44 26 51 30V48L58 43V25Q58 18 65 23V32L73 25Q80 22 81 29L65 43V56Q78 57 78 46V40Q78 33 85 37V49Q85 68 69 69V103Z",
		renderDetails: ({ palette }) => (
			<g fill={palette.accent}>
				<circle cx="38" cy="91" r="2" />
				<circle cx="62" cy="93" r="2.6" />
				<circle cx="55" cy="98" r="1.7" />
				<circle cx="35" cy="51" r="1.7" />
				<circle cx="61" cy="55" r="2" />
			</g>
		),
	}),
	crewShape({
		id: "jelly",
		label: "Jelly",
		series: "ocean",
		description: "A pale blue jellyfish with ribbon-like tentacles.",
		palette: {
			body: "#9ad6df",
			background: "#eee5f5",
			accent: "#6bafc8",
			clothing: "#a382b8",
		},
		bodyMotion: "float",
		anchors: {
			head: { x: 50, y: 30, width: 45 },
			eyes: { left: [39, 52], right: [61, 52], radius: 3.6 },
			mouth: [50, 63],
			neck: { y: 68, bottom: 79 },
		},
		silhouette:
			"M13 69C13 43 28 27 50 27S87 43 87 69Q87 77 77 75Q67 83 59 76Q50 82 41 76Q32 83 23 75Q13 77 13 69Z",
		renderDetails: ({ palette }) => (
			<>
				<g
					fill="none"
					stroke={palette.accent}
					strokeWidth="5"
					strokeLinecap="round"
				>
					{[
						[27, "M27 75Q20 85 29 92Q33 97 29 102"],
						[43, "M43 76Q50 87 42 100"],
						[59, "M59 76Q52 87 61 100"],
						[75, "M75 75Q82 85 73 94Q69 99 73 104"],
					].map(([x, d], index) => (
						<BotMotion
							key={x}
							kind="sway"
							origin={[Number(x), 76]}
							duration={3600}
							delay={index * -650}
							amount={7}
						>
							<path d={String(d)} />
						</BotMotion>
					))}
				</g>
				<path
					d="M27 47Q33 36 43 35"
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
		id: "pearl",
		faceColor: "sclera",
		label: "Pearl",
		series: "ocean",
		description: "A cream pearl nestled inside a scalloped pink shell.",
		palette: {
			body: "#d995b2",
			background: "#e4e9f6",
			accent: "#f2bfd0",
			sclera: "#fff7e9",
			clothing: "#7d85aa",
		},
		cheeks: true,
		anchors: {
			head: { x: 50, y: 42, width: 38 },
			eyes: { left: [41, 64], right: [59, 64], radius: 3.4 },
			mouth: [50, 76],
			neck: { y: 84, bottom: 100 },
		},
		silhouette:
			"M50 95C29 91 12 72 10 55Q8 41 20 43Q20 27 33 33Q38 20 50 31Q62 20 67 33Q80 27 80 43Q92 41 90 55C88 72 71 91 50 95Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M20 47L39 79M33 36L44 75M50 33V73M67 36L56 75M80 47L61 79"
					fill="none"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<circle cx="50" cy="66" r="24" fill={palette.sclera} />
				<path
					d="M17 80Q50 98 83 80Q77 103 50 103Q23 103 17 80Z"
					fill={palette.accent}
				/>
				<path
					d="M27 89Q50 101 73 89"
					fill="none"
					stroke={palette.body}
					strokeWidth="2"
				/>
			</>
		),
	}),
];
