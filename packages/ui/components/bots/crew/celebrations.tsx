import type { BotShapeDefinition } from "../types";
import { crewShape, radialPath } from "./helpers";
import { BotMotion } from "./motion";

const confettiBody = "M22 44Q50 33 78 44L53 103Q50 108 47 103Z";

export const celebrationsShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "noelle",
		label: "Noelle",
		series: "celebrations",
		description: "A Christmas fir with golden garlands and tiny red ornaments.",
		palette: {
			body: "#51866b",
			background: "#e5eedf",
			accent: "#e8c772",
			cheek: "#cd6b65",
			clothing: "#8b6074",
			sclera: "#fff1d5",
		},
		silhouette: "M50 20L71 48H63L83 75H72L94 103H6L28 75H17L37 48H29Z",
		anchors: {
			head: { x: 50, y: 27, width: 31 },
			eyes: { left: [40, 66], right: [60, 66], radius: 3.7 },
			mouth: [50, 80],
			neck: { y: 86, bottom: 105 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<path d={radialPath(50, 20, [7, 3], 5)} fill={palette.accent} />
				)}
				<path
					d="M36 46Q50 55 64 46M27 84Q50 98 73 84"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<g fill={palette.cheek}>
					<circle cx="32" cy="62" r="3" />
					<circle cx="69" cy="91" r="3" />
					<circle cx="37" cy="95" r="2.6" />
				</g>
			</>
		),
	}),
	crewShape({
		id: "jacklet",
		label: "Jacklet",
		series: "celebrations",
		description: "A round Halloween pumpkin with a curly green stem.",
		palette: {
			body: "#e89a52",
			background: "#eee1ed",
			accent: "#64865c",
			clothing: "#aa6a48",
			ink: "#493440",
			sclera: "#ffe0a4",
		},
		silhouette:
			"M19 56C21 42 33 34 44 39Q50 34 56 39C67 34 79 42 81 56C95 57 95 91 81 99Q50 109 19 99C5 91 5 57 19 56Z",
		anchors: {
			head: { x: 50, y: 38, width: 48 },
			eyes: { left: [38, 66], right: [62, 66], radius: 3.8 },
			mouth: [50, 80],
			neck: { y: 86, bottom: 105 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				<path
					d="M33 43Q23 68 32 100M67 43Q77 68 68 100"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="2"
					opacity=".45"
				/>
				<path
					d="M49 42V54M49 90V96"
					stroke={palette.sclera}
					strokeWidth="2"
					opacity=".25"
				/>
				{!equipment?.head && (
					<>
						<path
							d="M48 40Q43 30 51 25L57 28Q49 31 54 40Z"
							fill={palette.accent}
						/>
						<path
							d="M53 35Q67 25 68 34Q66 39 62 35"
							fill="none"
							stroke={palette.accent}
							strokeWidth="2"
							strokeLinecap="round"
						/>
					</>
				)}
			</>
		),
	}),
	crewShape({
		id: "paska",
		label: "Paska",
		series: "celebrations",
		description: "A pastel Easter egg decorated with lilac waves and dots.",
		palette: {
			body: "#e7edc9",
			background: "#f2e4ec",
			accent: "#af8bc9",
			clothing: "#7b9db6",
			cheek: "#e5ada0",
			sclera: "#fff8eb",
		},
		silhouette:
			"M50 23C69 23 87 63 85 82Q83 105 50 105Q17 105 15 82C13 63 31 23 50 23Z",
		cheeks: true,
		anchors: {
			head: { x: 50, y: 28, width: 33 },
			eyes: { left: [39, 67], right: [61, 67], radius: 3.7 },
			mouth: [50, 80],
			neck: { y: 86, bottom: 105 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M29 45l7 -4 7 4 7 -4 7 4 7 -4 7 4M20 90q5 -7 10 0t10 0t10 0t10 0t10 0t10 0"
					fill="none"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinejoin="round"
				/>
				<g fill={palette.clothing}>
					<circle cx="39" cy="53" r="1.8" />
					<circle cx="50" cy="54" r="1.8" />
					<circle cx="61" cy="53" r="1.8" />
				</g>
			</>
		),
	}),
	crewShape({
		id: "confetti",
		label: "Confetti",
		series: "celebrations",
		description: "A New Year party popper with bright paper streamers.",
		palette: {
			body: "#a38aca",
			background: "#f1e4f2",
			accent: "#e8bb62",
			clothing: "#648ca2",
			cheek: "#d4809d",
			sclera: "#f6e5c7",
		},
		silhouette: confettiBody,
		anchors: {
			head: { x: 50, y: 41, width: 51 },
			eyes: { left: [40, 61], right: [60, 61], radius: 3.4 },
			mouth: [50, 75],
			neck: { width: 23, y: 81, bottom: 105 },
		},
		renderDetails: ({ palette, equipment, idPrefix }) => (
			<>
				<ellipse cx="50" cy="44" rx="28" ry="7" fill={palette.accent} />
				<defs>
					<clipPath id={`${idPrefix}-confetti-cone`}>
						<path d={confettiBody} />
					</clipPath>
				</defs>
				<ellipse
					cx="50"
					cy="43"
					rx="22"
					ry="3"
					fill={palette.ink}
					opacity=".3"
				/>
				<g clipPath={`url(#${idPrefix}-confetti-cone)`}>
					<path
						d="M38 84L59 92M43 95L54 99"
						stroke={palette.sclera}
						strokeWidth="4"
					/>
				</g>
				{!equipment?.head && (
					<g fill="none" strokeWidth="3" strokeLinecap="round">
						<BotMotion
							kind="flutter"
							origin={[31, 29]}
							duration={4200}
							amount={4}
						>
							<path d="M31 29Q13 32 17 17" stroke={palette.cheek} />
						</BotMotion>
						<BotMotion
							kind="flutter"
							origin={[67, 31]}
							duration={4600}
							delay={-1700}
							amount={-4}
						>
							<path d="M67 31Q83 27 79 15" stroke={palette.cheek} />
						</BotMotion>
						<BotMotion kind="bob" duration={4000} amount={2}>
							<path d="M49 29L44 18L52 12" stroke={palette.accent} />
							<path
								d="M29 15L31 18M68 16L65 19M87 39L91 36"
								stroke={palette.clothing}
							/>
						</BotMotion>
					</g>
				)}
			</>
		),
	}),
	crewShape({
		id: "hong",
		label: "Hong",
		series: "celebrations",
		description: "A red Lunar New Year lantern with gold ribs and a tassel.",
		palette: {
			body: "#cf655e",
			background: "#f8e5d8",
			accent: "#e7bd62",
			clothing: "#8f5560",
			ink: "#492e3d",
			sclera: "#fff0c9",
		},
		silhouette:
			"M35 28H65V36Q83 42 83 59Q83 76 65 82V88H35V82Q17 76 17 59Q17 42 35 36Z",
		anchors: {
			head: { x: 50, y: 29, width: 36 },
			eyes: { left: [39, 60], right: [61, 60], radius: 3.6 },
			mouth: [50, 73],
			neck: { y: 80, bottom: 89 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				<path d="M35 28H65V36H35ZM35 82H65V88H35Z" fill={palette.accent} />
				<path
					d="M34 38Q23 59 34 80M66 38Q77 59 66 80"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2"
					opacity=".7"
				/>
				<BotMotion kind="sway" origin={[50, 88]} duration={4600} amount={8}>
					<path
						d="M46 89V96M50 89V98M54 89V96"
						stroke={palette.accent}
						strokeWidth="2"
						strokeLinecap="round"
					/>
				</BotMotion>
				{!equipment?.head && (
					<path
						d="M42 28V22Q50 13 58 22V28"
						fill="none"
						stroke={palette.clothing}
						strokeWidth="2"
					/>
				)}
			</>
		),
	}),
	crewShape({
		id: "diya",
		label: "Diya",
		series: "celebrations",
		description: "A terracotta Diwali lamp holding a golden flame.",
		palette: {
			body: "#b87953",
			background: "#f5e5d2",
			accent: "#ebbd66",
			clothing: "#8e657e",
			ink: "#49303b",
			sclera: "#fff0c4",
		},
		silhouette: "M12 60Q50 48 88 60Q83 94 50 101Q17 94 12 60Z",
		anchors: {
			head: { x: 50, y: 54, width: 58 },
			eyes: { left: [38, 73], right: [62, 73], radius: 3.4 },
			mouth: [50, 82],
			neck: { width: 45, y: 87, bottom: 101 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				<ellipse cx="50" cy="60" rx="37" ry="8" fill={palette.accent} />
				<ellipse
					cx="50"
					cy="60"
					rx="29"
					ry="4"
					fill={palette.ink}
					opacity=".2"
				/>
				{!equipment?.head && (
					<BotMotion
						kind="flicker"
						origin={[50, 59]}
						duration={2800}
						amount={6}
					>
						<path
							d="M50 22Q72 48 56 58Q42 63 38 48Q35 36 50 22Z"
							fill={palette.accent}
						/>
						<path d="M50 39Q61 53 51 59Q41 55 50 39Z" fill={palette.sclera} />
					</BotMotion>
				)}
				<path
					d="M23 81L28 85M72 85L77 81M32 91L36 94M64 94L68 91"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
];
