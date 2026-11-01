import type { BotShapeDefinition } from "../types";
import { crewShape } from "./helpers";
import { BotMotion } from "./motion";

export const officeShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "clip",
		label: "Clip",
		series: "office",
		description: "A silver paperclip with big, curious eyes.",
		palette: {
			body: "#9aaebf",
			background: "#e8edf8",
			accent: "#dce7ef",
			clothing: "#6489af",
		},
		eyeStyle: "round",
		silhouette:
			"M62 76V44C62 21 28 21 28 44V80C28 109 76 109 76 80V44H68V80C68 98 36 98 36 80V44C36 32 54 32 54 44V76C54 83 46 83 46 76V48H38V76C38 94 62 94 62 76Z",
		anchors: {
			eyes: { left: [39, 52], right: [61, 52], radius: 7.5 },
			mouth: [50, 70],
			head: { x: 48, y: 29, width: 40 },
			neck: { y: 82, bottom: 103 },
			hand: [30, 84],
		},
		renderDetails: ({ palette }) => (
			<path
				d="M32 74V44C32 27 58 27 58 44M72 46V80Q72 98 53 98"
				fill="none"
				stroke={palette.accent}
				strokeWidth="2"
				strokeLinecap="round"
			/>
		),
	}),
	crewShape({
		id: "jot",
		label: "Jot",
		series: "office",
		description: "A yellow pencil with a pink eraser and a sharp graphite tip.",
		palette: {
			body: "#f3c553",
			background: "#f8eddb",
			accent: "#ec9ca6",
			clothing: "#9eafb9",
			sclera: "#f7dfb4",
		},
		cheeks: true,
		silhouette: "M32 27Q32 20 39 20H61Q68 20 68 27V79L50 104L32 79Z",
		anchors: {
			eyes: { left: [42, 58], right: [58, 58], radius: 3 },
			mouth: [50, 70],
			head: { x: 50, y: 21, width: 31 },
			neck: { y: 76, bottom: 103 },
			hand: [32, 80],
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M32 34V27Q32 20 39 20H61Q68 20 68 27V34Z"
					fill={palette.accent}
				/>
				<path d="M32 34H68V42H32Z" fill={palette.clothing} />
				<path d="M36 44V76M64 44V76" stroke={palette.sclera} strokeWidth="2" />
				<path d="M32 79H68L50 104Z" fill={palette.sclera} />
				<path d="M43 94H57L50 104Z" fill={palette.ink} />
			</>
		),
	}),
	crewShape({
		id: "memo",
		label: "Memo",
		series: "office",
		description: "A lemon-yellow sticky note with a curled corner.",
		palette: {
			body: "#f7dc72",
			background: "#e3efe7",
			accent: "#e4b94d",
			sclera: "#fff3b0",
			clothing: "#76a99a",
		},
		cheeks: true,
		silhouette: "M19 31H81V78L60 99H19Z",
		anchors: {
			head: { x: 50, y: 32, width: 49 },
			eyes: { left: [39, 62], right: [61, 62], radius: 3.4 },
			mouth: [50, 74],
			neck: { y: 83, bottom: 102 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M19 31H81V41H19Z" fill={palette.accent} opacity=".35" />
				<path d="M60 99V78H81Z" fill={palette.accent} />
				<BotMotion kind="flutter" origin={[60, 78]} amount={3} duration={4600}>
					<path d="M60 99V78H81Z" fill={palette.sclera} />
					<path
						d="M60 99V78H81"
						fill="none"
						stroke={palette.accent}
						strokeWidth="1.4"
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "rubi",
		label: "Rubi",
		series: "office",
		description: "A rosy eraser tucked into a cream paper sleeve.",
		palette: {
			body: "#ed9ca9",
			background: "#f6e5ec",
			accent: "#c87e95",
			sclera: "#fff0db",
			clothing: "#8e9cbe",
		},
		cheeks: true,
		silhouette: "M29 32H78Q84 32 82 39L69 101H14Z",
		anchors: {
			head: { x: 54, y: 33, width: 43 },
			eyes: { left: [38, 65], right: [60, 65], radius: 3.5 },
			mouth: [49, 78],
			neck: { y: 85, bottom: 103 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M24 52H79L69 101H14Z" fill={palette.sclera} />
				<path
					d="M36 38H73"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".6"
				/>
				<path d="M20 90H71" stroke={palette.accent} strokeWidth="2" />
			</>
		),
	}),
	crewShape({
		id: "ruler",
		label: "Ruler",
		series: "office",
		description:
			"A wooden ruler with little measurement marks down both sides.",
		palette: {
			body: "#e9bb75",
			background: "#e5edf5",
			accent: "#a47647",
			clothing: "#658ca5",
		},
		mouthStyle: "line",
		silhouette: "M32 21H68Q73 21 73 26V102H27V26Q27 21 32 21Z",
		anchors: {
			head: { x: 50, y: 22, width: 40 },
			eyes: { left: [42, 62], right: [58, 62], radius: 3 },
			mouth: [50, 75],
			hand: [28, 84],
		},
		renderDetails: ({ palette }) => (
			<>
				<circle cx="50" cy="32" r="4" fill={palette.background} />
				<g stroke={palette.accent} strokeWidth="1.5">
					{[43, 52, 61, 70, 79, 88, 97].map((y, index) => (
						<path
							key={y}
							d={`M28 ${y}h${index % 2 ? 5 : 8}M72 ${y}h-${index % 2 ? 5 : 8}`}
						/>
					))}
				</g>
			</>
		),
	}),
	crewShape({
		id: "binder",
		label: "Binder",
		series: "office",
		description: "A blue spiral notebook with a bookmark peeking out below.",
		palette: {
			body: "#7eacd1",
			background: "#e4edf5",
			accent: "#e9bc69",
			clothing: "#52749a",
		},
		silhouette: "M26 29H76Q82 29 82 35V103H22V35Q22 29 26 29Z",
		anchors: {
			head: { x: 52, y: 30, width: 46 },
			eyes: { left: [44, 65], right: [64, 65], radius: 3.4 },
			mouth: [54, 78],
			neck: { y: 86, bottom: 105 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M29 30V103" stroke={palette.clothing} strokeWidth="2" />
				<BotMotion kind="sway" origin={[64, 84]} amount={4} duration={4800}>
					<path d="M60 84H68V101L64 97L60 101Z" fill={palette.accent} />
				</BotMotion>
				<path
					d="M40 42H70"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".65"
				/>
				<g
					fill="none"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
				>
					{[40, 54, 68, 82, 96].map((y) => (
						<path key={y} d={`M25 ${y}H18Q14 ${y - 4} 18 ${y - 7}H24`} />
					))}
				</g>
			</>
		),
	}),
	crewShape({
		id: "klick",
		label: "Klick",
		series: "office",
		description: "A computer mouse with a cord and an orange button.",
		palette: {
			body: "#f0ede7",
			background: "#ffe4d1",
			accent: "#ff513b",
			cheek: "#fac1ab",
			clothing: "#c9c1b7",
			ink: "#443139",
		},
		cheeks: true,
		mouthScale: 1.3,
		silhouette:
			"M27 52C27 34 37 25 50 25S74 34 74 52V70C74 88 64 96 50 96S27 88 27 70Z",
		anchors: {
			eyes: { left: [43, 65], right: [59, 65], radius: 3.8 },
			mouth: [51, 77],
			head: { x: 50, y: 29, width: 32 },
			neck: { y: 82, bottom: 99 },
			hand: [29, 85],
		},
		renderDetails: ({ palette }) => (
			<>
				<ellipse
					cx="50"
					cy="97"
					rx="20"
					ry="3"
					fill={palette.cheek}
					opacity="0.5"
				/>
				<path
					d="M50 27V17C50 4 43 2 35 5S22 6 17 0"
					stroke={palette.clothing}
					strokeWidth="2.5"
					fill="none"
					strokeLinecap="round"
				/>
				<path d="M27 49C29 34 38 25 49 25V53Z" fill={palette.accent} />
				<path
					d="M27 49Q50 55 74 49"
					stroke={palette.clothing}
					strokeWidth="2.2"
					fill="none"
				/>
				<rect x="47" y="31" width="6" height="13" rx="3" fill={palette.ink} />
			</>
		),
	}),
];
