import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

export const adventureShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "scout",
		label: "Scout",
		series: "adventure",
		description: "A pocket compass with a red north-pointing needle.",
		palette: {
			body: "#d7af68",
			background: "#e8ecdd",
			accent: "#d97460",
			sclera: "#fff4d7",
			clothing: "#738579",
		},
		silhouette: ovalPath(50, 66, 35),
		anchors: {
			head: { x: 50, y: 31, width: 45 },
			eyes: { left: [39, 69], right: [61, 69], radius: 3.5 },
			mouth: [50, 81],
			neck: { y: 86, bottom: 103 },
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<circle
						cx="50"
						cy="26"
						r="7"
						fill="none"
						stroke={palette.body}
						strokeWidth="4"
					/>
				)}
				<circle cx="50" cy="66" r="29" fill={palette.sclera} />
				<BotMotion kind="pendulum" origin={[50, 55]} amount={9} duration={5400}>
					<path d="M50 41L44 58L50 55L56 58Z" fill={palette.accent} />
				</BotMotion>
				<circle cx="50" cy="55" r="1.8" fill={palette.clothing} />
				<path
					d="M24 66H29M71 66H76M50 88V93"
					stroke={palette.clothing}
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "summit",
		label: "Summit",
		series: "adventure",
		description: "A blue mountain with a snowy peak and a pine at its foot.",
		palette: {
			body: "#849fb7",
			background: "#e2edf2",
			accent: "#4f867c",
			clothing: "#596f8e",
			sclera: "#f9fbf7",
		},
		silhouette: "M44 23Q50 14 56 23L94 102H6Z",
		anchors: {
			head: { x: 50, y: 24, width: 24 },
			eyes: { left: [39, 69], right: [61, 69], radius: 3.7 },
			mouth: [50, 81],
			neck: { y: 86, bottom: 105 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M44 23Q50 14 56 23L69 50L57 44L50 51L43 43L32 48Z"
					fill={palette.sclera}
				/>
				<path d="M21 99V85" stroke={palette.clothing} strokeWidth="3" />
				<path d="M21 74L11 90H31Z" fill={palette.accent} />
				<path
					d="M75 92L80 82L89 100H71Z"
					fill={palette.clothing}
					opacity=".3"
				/>
			</>
		),
	}),
	crewShape({
		id: "camp",
		label: "Camp",
		series: "adventure",
		description: "An ochre camping tent with a tied-back doorway.",
		palette: {
			body: "#e5ae5d",
			background: "#e5eddf",
			accent: "#f6d89c",
			clothing: "#79674e",
		},
		cheeks: true,
		silhouette: "M50 26L94 97H6Z",
		anchors: {
			head: { x: 50, y: 29, width: 28 },
			eyes: { left: [39, 66], right: [61, 66], radius: 3.5 },
			mouth: [50, 78],
			neck: { y: 85, bottom: 100 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M50 27L25 97H6Z" fill={palette.accent} />
				<path d="M50 27L75 97H94Z" fill={palette.clothing} opacity=".15" />
				<path d="M45 85H55L62 97H38Z" fill={palette.clothing} />
				<path
					d="M50 27V20M17 88L7 99M83 88L93 99"
					stroke={palette.clothing}
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "beacon",
		label: "Beacon",
		series: "adventure",
		description: "A striped lighthouse with a little glowing lantern.",
		palette: {
			body: "#fff3de",
			background: "#dfeaf0",
			accent: "#dc7965",
			clothing: "#597b91",
			cheek: "#f4cb73",
		},
		silhouette: "M34 30L50 18L66 30V44H62L72 103H28L38 44H34Z",
		anchors: {
			head: { x: 50, y: 22, width: 29 },
			eyes: { left: [41, 69], right: [59, 69], radius: 3.1 },
			mouth: [50, 80],
			neck: { y: 87, bottom: 105 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M34 30L50 18L66 30Z" fill={palette.accent} />
				<BotMotion kind="flicker" origin={[50, 37]} duration={5200}>
					<path d="M40 32H60V42H40Z" fill={palette.cheek} />
					<path
						d="M39 35L14 26V45L39 39ZM61 35L86 26V45L61 39Z"
						fill={palette.cheek}
						opacity=".3"
					/>
				</BotMotion>
				<path
					d="M50 32V42M32 44H68"
					stroke={palette.clothing}
					strokeWidth="2"
				/>
				<path
					d="M37 49H63L65 59H35ZM30 90H70L72 103H28Z"
					fill={palette.accent}
				/>
			</>
		),
	}),
	crewShape({
		id: "skipper",
		label: "Skipper",
		series: "adventure",
		description: "A coral sailboat beneath a cream-colored sail.",
		palette: {
			body: "#e58c77",
			background: "#dbeef3",
			accent: "#fff2cf",
			clothing: "#5b91a9",
		},
		silhouette: "M47 21H51V63H91L78 94Q50 105 22 94L9 63H47Z",
		anchors: {
			head: { x: 50, y: 27, width: 29 },
			eyes: { left: [39, 76], right: [61, 76], radius: 3.5 },
			mouth: [50, 87],
			neck: { y: 92, bottom: 104 },
		},
		renderDetails: ({ palette }) => (
			<>
				<BotMotion kind="flutter" origin={[45, 58]} amount={2} duration={4200}>
					<path d="M45 23L16 58H45Z" fill={palette.accent} />
				</BotMotion>
				<BotMotion
					kind="flutter"
					origin={[54, 58]}
					amount={3}
					duration={4200}
					delay={-900}
				>
					<path d="M54 33V58H79Z" fill={palette.accent} />
				</BotMotion>
				<path d="M50 22V62" stroke={palette.clothing} strokeWidth="2" />
				<path d="M12 67H88" stroke={palette.accent} strokeWidth="3" />
				<BotMotion kind="bob" origin={[50, 95]} amount={1} duration={4200}>
					<path
						d="M6 95Q17 91 28 95T50 95T72 95T94 95V105H6Z"
						fill={palette.clothing}
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "globe",
		label: "Globe",
		series: "adventure",
		description: "A desk globe with green continents and a brass stand.",
		palette: {
			body: "#90c6dd",
			background: "#e7efdf",
			accent: "#7fa77a",
			clothing: "#ad8a54",
		},
		silhouette: ovalPath(50, 61, 30),
		anchors: {
			head: { x: 50, y: 32, width: 39 },
			eyes: { left: [40, 62], right: [60, 62], radius: 3.6 },
			mouth: [50, 75],
			neck: { y: 82, bottom: 92 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M30 26C87 14 112 94 50 96V102M35 103H65"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<g fill={palette.accent}>
					<path d="M26 44L35 36L49 32L53 41L46 49L37 47L32 55L23 52Z" />
					<path d="M69 44L77 50L80 61L73 69L67 65L70 57L64 51Z" />
					<path d="M27 70L34 77L37 86L30 82L24 75Z" />
					<path d="M58 81L68 77L70 82L61 88Z" />
				</g>
			</>
		),
	}),
];
