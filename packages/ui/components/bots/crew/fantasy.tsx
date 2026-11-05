import type { BotShapeDefinition } from "../types";
import { crewShape } from "./helpers";
import { BotMotion } from "./motion";

export const fantasyShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "wisp",
		label: "Wisp",
		series: "fantasy",
		description: "A floating ghost with a scalloped hem and shy cheeks.",
		palette: {
			body: "#fffaf0",
			background: "#e7dcf1",
			accent: "#d6c9eb",
			clothing: "#8b79b0",
			cheek: "#efb7c6",
		},
		cheeks: true,
		bodyMotion: "float",
		anchors: {
			head: { x: 50, y: 31, width: 43 },
			eyes: { left: [39, 58], right: [61, 58], radius: 4 },
			mouth: [50, 71],
			neck: { y: 79, bottom: 98 },
		},
		silhouette:
			"M20 65C20 42 31 28 50 28S80 42 80 65Q78 83 88 94Q77 100 69 91Q61 104 50 94Q39 104 31 91Q23 100 12 94Q22 83 20 65Z",
		renderDetails: ({ palette }) => (
			<g fill={palette.accent}>
				<path className="fl-bot-star" d="M17 32l2 5 5 2-5 2-2 5-2-5-5-2 5-2Z" />
				<path
					className="fl-bot-star fl-bot-star-1"
					d="M83 49l1.5 3 3 1.5-3 1.5-1.5 3-1.5-3-3-1.5 3-1.5Z"
				/>
			</g>
		),
	}),
	crewShape({
		id: "draco",
		label: "Draco",
		series: "fantasy",
		description: "A mint dragon with small wings and golden horns.",
		palette: {
			body: "#79b9a0",
			background: "#e5ecdb",
			accent: "#f0cb80",
			sclera: "#c6dfb1",
			clothing: "#627da1",
		},
		anchors: {
			head: { x: 50, y: 38, width: 42 },
			eyes: { left: [39, 60], right: [61, 60], radius: 3.8 },
			mouth: [50, 75],
			neck: { y: 87, bottom: 104, width: 46 },
		},
		silhouette:
			"M22 63C22 43 34 34 50 34S78 43 78 63L80 77Q83 92 75 104H25Q17 92 20 77Z",
		renderBackDetails: ({ palette }) => (
			<>
				<BotMotion kind="flutter" origin={[25, 73]} duration={3800} amount={7}>
					<path
						d="M27 64Q17 54 10 43Q8 60 12 77Q18 71 28 78Z"
						fill={palette.body}
					/>
					<path
						d="M25 66L13 51Q12 62 15 71Q20 69 25 74Z"
						fill={palette.sclera}
					/>
					<path
						d="M13 51Q19 62 25 74"
						fill="none"
						stroke={palette.body}
						strokeWidth="1.3"
					/>
				</BotMotion>
				<BotMotion kind="flutter" origin={[75, 73]} duration={3800} amount={-7}>
					<path
						d="M73 64Q83 54 90 43Q92 60 88 77Q82 71 72 78Z"
						fill={palette.body}
					/>
					<path
						d="M75 66L87 51Q88 62 85 71Q80 69 75 74Z"
						fill={palette.sclera}
					/>
					<path
						d="M87 51Q81 62 75 74"
						fill="none"
						stroke={palette.body}
						strokeWidth="1.3"
					/>
				</BotMotion>
			</>
		),
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M31 40Q26 31 29 23Q38 27 40 37ZM60 37Q62 27 71 23Q74 31 69 40Z"
					fill={palette.accent}
				/>
				<path
					d="M34 71Q36 64 43 65H57Q64 64 66 71Q71 85 50 85Q29 85 34 71Z"
					fill={palette.sclera}
				/>
				<path d="M46 43L50 37L54 43Z" fill={palette.accent} />
			</>
		),
		renderFaceDetails: ({ palette }) => (
			<g fill={palette.body}>
				<ellipse cx="43" cy="69" rx="1.7" ry="1.2" />
				<ellipse cx="57" cy="69" rx="1.7" ry="1.2" />
			</g>
		),
	}),
	crewShape({
		id: "rune",
		label: "Rune",
		series: "fantasy",
		description: "A plum spellbook with gold corners and a ribbon bookmark.",
		palette: {
			body: "#a183bb",
			background: "#eee4f5",
			accent: "#efd18b",
			clothing: "#675184",
			sclera: "#fff2db",
		},
		anchors: {
			head: { x: 51, y: 34, width: 50 },
			eyes: { left: [42, 64], right: [64, 64], radius: 3.6 },
			mouth: [53, 76],
			neck: { y: 83, bottom: 103 },
		},
		silhouette: "M23 32H77Q83 32 83 38V100H23Q15 100 15 92V41Q15 32 23 32Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M21 38H28V91H21Q17 93 21 96H79V101H21Q14 100 14 93V42Q14 36 21 38Z"
					fill={palette.clothing}
				/>
				<path d="M25 91H78V96H25Z" fill={palette.sclera} />
				<path d="M61 92H69V106L65 102L61 106Z" fill={palette.accent} />
				<path
					d="M32 37H42L32 47ZM69 37H78V46ZM78 78V87H69Z"
					fill={palette.accent}
				/>
				<path d="M53 41l2 5 5 2-5 2-2 5-2-5-5-2 5-2Z" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "glimmer",
		label: "Glimmer",
		series: "fantasy",
		description: "A faceted lilac crystal with tiny floating sparkles.",
		palette: {
			body: "#c3a2e6",
			background: "#e5e9f8",
			accent: "#9a7bc4",
			sclera: "#eadbf8",
			clothing: "#687ba7",
		},
		anchors: {
			head: { x: 50, y: 34, width: 32 },
			eyes: { left: [41, 64], right: [59, 64], radius: 3.3 },
			mouth: [50, 76],
			neck: { y: 83, bottom: 101 },
		},
		silhouette: "M50 20L80 48V82L50 102L20 82V48Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M50 20L35 49L20 48ZM50 20L65 49L80 48Z"
					fill={palette.sclera}
				/>
				<path
					d="M20 48L35 49V82L50 102L20 82ZM80 48L65 49V82L50 102L80 82Z"
					fill={palette.accent}
					opacity=".65"
				/>
				<path
					d="M35 49H65M35 82L50 102L65 82"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="1.2"
					opacity=".8"
				/>
				<g fill={palette.accent}>
					<path
						className="fl-bot-star"
						d="M17 30l2 4 4 2-4 2-2 4-2-4-4-2 4-2Z"
					/>
					<path
						className="fl-bot-star fl-bot-star-1"
						d="M85 65l1.5 3 3 1.5-3 1.5-1.5 3-1.5-3-3-1.5 3-1.5Z"
					/>
				</g>
			</>
		),
	}),
	crewShape({
		id: "wish",
		label: "Wish",
		series: "fantasy",
		description:
			"A golden wishing lamp with a curled handle and wisps of magic.",
		palette: {
			body: "#e7bb65",
			background: "#ece1f4",
			accent: "#b98b46",
			sclera: "#fff0bd",
			clothing: "#8070a4",
		},
		anchors: {
			head: { x: 48, y: 45, width: 33 },
			eyes: { left: [38, 67], right: [58, 67], radius: 3.4 },
			mouth: [48, 78],
			neck: { y: 84, bottom: 103 },
			hand: [22, 83],
		},
		silhouette:
			"M27 55Q48 46 69 55L89 43L95 44Q87 67 75 72Q73 87 57 91V97H66V103H30V97H39V91Q22 87 20 74Q17 60 27 55Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M24 58C4 44 1 78 22 78"
					fill="none"
					stroke={palette.accent}
					strokeWidth="5"
				/>
				<path d="M31 54Q48 37 65 54Z" fill={palette.accent} />
				<circle cx="48" cy="43" r="4" fill={palette.body} />
				<BotMotion kind="rise" origin={[88, 37]} duration={4700} amount={5}>
					<path
						d="M88 37Q78 29 85 22Q91 15 83 12"
						fill="none"
						stroke={palette.clothing}
						strokeWidth="3"
						opacity=".55"
						strokeLinecap="round"
					/>
				</BotMotion>
				<path d="M35 91H61" stroke={palette.sclera} strokeWidth="2" />
			</>
		),
	}),
	crewShape({
		id: "keep",
		label: "Keep",
		series: "fantasy",
		description: "A little blue castle with battlements and a red flag.",
		palette: {
			body: "#a1b7ce",
			background: "#e7ecf2",
			accent: "#d47e88",
			clothing: "#687e9b",
			sclera: "#d6e1eb",
		},
		anchors: {
			head: { x: 50, y: 40, width: 42 },
			eyes: { left: [39, 64], right: [61, 64], radius: 3.5 },
			mouth: [50, 76],
			neck: { y: 83, bottom: 104 },
		},
		silhouette: "M15 104V33H23V42H30V33H38V49H62V33H70V42H77V33H85V104Z",
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<>
						<path
							d="M50 49V19"
							stroke={palette.clothing}
							strokeWidth="2"
							strokeLinecap="round"
						/>
						<BotMotion
							kind="flutter"
							origin={[51, 26]}
							duration={3500}
							amount={5}
						>
							<path d="M51 20H68L63 26L68 32H51Z" fill={palette.accent} />
						</BotMotion>
					</>
				)}
				<path
					d="M15 49H38M62 49H85M15 79H27M73 79H85M15 94H23M77 94H85"
					stroke={palette.sclera}
					strokeWidth="2"
				/>
				<path
					d="M40 104V92Q40 82 50 82Q60 82 60 92V104Z"
					fill={palette.clothing}
				/>
				<path d="M50 88V104" stroke={palette.sclera} strokeWidth="1.5" />
			</>
		),
	}),
];
