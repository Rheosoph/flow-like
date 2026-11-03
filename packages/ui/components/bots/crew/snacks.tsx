import type { BotShapeDefinition } from "../types";
import { crewShape } from "./helpers";
import { BotMotion } from "./motion";

const puriBody =
	"M28 45C28 38 72 38 72 45L84 87C88 96 70 100 50 100S12 96 16 87Z";

export const snacksShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "momo",
		label: "Momo",
		series: "snacks",
		description: "Dumpling with folded dough and rosy cheeks.",
		palette: {
			body: "#f7e7c9",
			background: "#f0cfc1",
			accent: "#cfb486",
			ink: "#3c3339",
			cheek: "#efb6a0",
			clothing: "#7e91a3",
		},
		anchors: { head: { x: 50, y: 36, width: 38 } },
		cheeks: true,
		silhouette:
			"M50 28C40 41 15 52 11 73C7 94 29 104 50 104S93 94 89 73C85 52 60 41 50 28Z",
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<g
						fill="none"
						stroke={palette.sclera}
						strokeWidth="2"
						strokeLinecap="round"
					>
						<BotMotion kind="rise" origin={[40, 23]} duration={4000} amount={5}>
							<path d="M40 23Q35 19 40 15Q44 12 41 9" />
						</BotMotion>
						<BotMotion
							kind="rise"
							origin={[60, 25]}
							duration={4000}
							delay={-2000}
							amount={5}
						>
							<path d="M60 25Q64 21 60 17Q56 14 59 11" />
						</BotMotion>
					</g>
				)}
				<g
					fill="none"
					stroke={palette.accent}
					strokeWidth="1.7"
					strokeLinecap="round"
				>
					<path d="M50 29L40 43M50 29L46 44M50 29L53 44M50 29L60 43" />
				</g>
			</>
		),
	}),
	crewShape({
		id: "nori",
		label: "Nori",
		series: "snacks",
		description: "Onigiri with a dark seaweed band.",
		palette: {
			body: "#fffdf7",
			background: "#e4ebdb",
			accent: "#2f4135",
			ink: "#34313a",
			cheek: "#efb8b3",
			clothing: "#899b6b",
		},
		anchors: { head: { x: 50, y: 31, width: 35 }, mouth: [50, 74] },
		cheeks: true,
		silhouette: "M43 27Q50 17 57 27L88 83Q96 100 79 104H21Q4 100 12 83Z",
		renderDetails: ({ palette }) => (
			<path d="M36 84H64V105H36Z" fill={palette.accent} />
		),
	}),
	crewShape({
		id: "toasty",
		label: "Toasty",
		series: "snacks",
		description: "Golden toast with a little pat of butter.",
		palette: {
			body: "#c58a43",
			background: "#f7e9d2",
			sclera: "#f6dca8",
			ink: "#403337",
			accent: "#f8df69",
			cheek: "#e6a477",
			clothing: "#7d938e",
		},
		anchors: { head: { x: 50, y: 30, width: 57 } },
		cheeks: true,
		silhouette:
			"M19 60C6 58 7 38 19 31C32 24 68 24 81 31C93 38 94 58 81 60V104H19Z",
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M24 56C14 53 15 40 25 36C37 31 63 31 75 36C85 40 86 53 76 56V104H24Z"
					fill={palette.sclera}
				/>
				<path d="M60 39L69 41L67 49L58 46Z" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "cupi",
		label: "Cupi",
		series: "snacks",
		description: "Cupcake with pink frosting and a cherry.",
		palette: {
			body: "#92c7de",
			background: "#fbe6ee",
			accent: "#f2afcb",
			cheek: "#dc5879",
			ink: "#30343e",
			clothing: "#5d9ebc",
			sclera: "#fff8f2",
		},
		anchors: {
			eyes: { left: [40, 72], right: [60, 72], radius: 3.4 },
			mouth: [50, 81],
			head: { x: 50, y: 34, width: 48 },
			neck: { y: 86, bottom: 100 },
		},
		cheeks: true,
		silhouette:
			"M19 60H81L76 105H24ZM18 63C9 63 10 47 20 46C18 31 34 27 42 34C47 22 62 24 66 36C78 32 87 41 82 48C92 48 91 62 81 63Z",
		renderDetails: ({ palette, equipment }) => (
			<>
				<g fill="none" stroke={palette.clothing} strokeWidth="2" opacity=".7">
					<path d="M29 62L33 104M42 62V67M58 62V67M43 96L44 104M57 96L56 104M71 62L67 104" />
				</g>
				<path
					d="M18 63C9 63 10 47 20 46C18 31 34 27 42 34C47 22 62 24 66 36C78 32 87 41 82 48C92 48 91 62 81 63Z"
					fill={palette.accent}
				/>
				{!equipment?.head && (
					<>
						<path
							d="M51 21Q50 15 56 13"
							fill="none"
							stroke={palette.clothing}
							strokeWidth="1.6"
							strokeLinecap="round"
						/>
						<circle cx="51" cy="22" r="5.3" fill={palette.cheek} />
					</>
				)}
			</>
		),
	}),
	crewShape({
		id: "puri",
		label: "Puri",
		series: "snacks",
		description: "Golden pudding with a caramel cap and a curl of cream.",
		palette: {
			body: "#f4d16c",
			background: "#f6e9d8",
			accent: "#b76b2f",
			ink: "#453536",
			cheek: "#eeb278",
			sclera: "#fff8e9",
			clothing: "#96735c",
		},
		anchors: {
			eyes: { left: [39, 68], right: [61, 68], radius: 3.7 },
			mouth: [50, 80],
			head: { x: 50, y: 42, width: 46 },
			neck: { y: 84, bottom: 100 },
		},
		cheeks: true,
		silhouette: puriBody,
		renderDetails: ({ palette, equipment, idPrefix }) => (
			<>
				<defs>
					<clipPath id={`${idPrefix}-puri-pudding`}>
						<path d={puriBody} />
					</clipPath>
				</defs>
				<g clipPath={`url(#${idPrefix}-puri-pudding)`}>
					<path
						d="M72 49L84 87Q87 95 69 98L65 54Z"
						fill={palette.accent}
						opacity=".12"
					/>
					<path
						d="M30 58L24 82"
						fill="none"
						stroke={palette.sclera}
						strokeWidth="3.5"
						strokeLinecap="round"
						opacity=".4"
					/>
					<path
						data-feature="puri-caramel"
						d="M20 36H80V48Q76 54 72 50Q68 60 63 51Q59 56 55 50Q51 61 46 51Q42 57 38 50Q34 56 30 50Q25 55 20 49Z"
						fill={palette.accent}
					/>
				</g>
				{!equipment?.head && (
					<path
						data-feature="puri-cream"
						d="M40 41C34 39 36 33 42 33C40 28 46 26 50 28C52 23 57 25 57 30C63 29 67 35 62 39C61 43 43 43 40 41Z"
						fill={palette.sclera}
					/>
				)}
			</>
		),
	}),
];
