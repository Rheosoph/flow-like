import type { BotShapeDefinition } from "../types";
import { crewShape, radialPath } from "./helpers";

export const techShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "nodi",
		label: "Nodi",
		series: "tech",
		description: "An orange node with round connector ears.",
		palette: {
			body: "#eb9c5e",
			background: "#ffead9",
			accent: "#f4c77f",
			clothing: "#53667d",
		},
		mouthStyle: "line",
		silhouette: "M18 51Q18 33 35 33H65Q82 33 82 51V105H18Z",
		anchors: {
			eyes: { left: [39, 64], right: [61, 64], radius: 3.2 },
			mouth: [50, 81],
			head: { x: 50, y: 34, width: 46 },
		},
		renderDetails: ({ palette }) => (
			<g fill={palette.sclera} stroke={palette.ink} strokeWidth="2.4">
				<circle cx="17" cy="65" r="5.6" />
				<circle cx="83" cy="65" r="5.6" />
			</g>
		),
	}),
	crewShape({
		id: "bit",
		label: "Bit",
		series: "tech",
		description: "A violet robot with a golden antenna.",
		palette: {
			body: "#787cdb",
			background: "#e4e5ff",
			accent: "#fac952",
			clothing: "#5058ab",
		},
		eyeStyle: "round",
		mouthStyle: "line",
		silhouette: "M16 57Q16 38 33 38H67Q84 38 84 57V104H16Z",
		anchors: {
			eyes: { left: [38, 66], right: [62, 66], radius: 7.2 },
			mouth: [50, 82],
			head: { x: 50, y: 38, width: 47 },
		},
		renderDetails: ({ palette }) => (
			<>
				<g
					className="fl-bot-crew-motion fl-bot-bit-antenna"
					data-motion-part="bit-antenna"
				>
					<path d="M50 38V23" stroke={palette.clothing} strokeWidth="3" />
					<circle cx="50" cy="22" r="5.5" fill={palette.accent} />
					<g
						className="fl-bot-crew-motion fl-bot-bit-signal"
						data-motion-part="bit-signal"
						fill="none"
						stroke={palette.accent}
						strokeWidth="1.5"
						strokeLinecap="round"
						opacity="0"
					>
						<path d="M42 18Q40 22 42 26M58 18Q60 22 58 26" />
						<path d="M37 15Q33 22 37 29M63 15Q67 22 63 29" opacity=".55" />
					</g>
				</g>
				<path d="M8 61H16V84H8Z M84 61H92V84H84Z" fill={palette.clothing} />
			</>
		),
	}),
	crewShape({
		id: "chip",
		label: "Chip",
		series: "tech",
		description: "A dark microchip with mint-green eyes and exposed pins.",
		palette: {
			body: "#303f47",
			background: "#dceee7",
			accent: "#becbd0",
			ink: "#94e4bd",
			sclera: "#94e4bd",
			clothing: "#94c5b3",
		},
		eyeStyle: "light",
		mouthStyle: "line",
		silhouette: "M25 34H75Q80 34 80 39V106H20V39Q20 34 25 34Z",
		anchors: {
			eyes: { left: [40, 60], right: [60, 60], radius: 3.2 },
			mouth: [50, 75],
			head: { x: 50, y: 34, width: 45 },
		},
		renderDetails: ({ palette }) => (
			<>
				<g fill={palette.accent}>
					{[30, 42, 54, 66].map((x) => (
						<rect key={`top-${x}`} x={x} y="26" width="4" height="8" />
					))}
					{[47, 59, 71, 83].map((y) => (
						<g key={`pin-${y}`}>
							<rect x="12" y={y} width="8" height="4" />
							<rect x="80" y={y} width="8" height="4" />
						</g>
					))}
				</g>
				{[
					"M14 49L9 52L16 55L11 58L14 61",
					"M86 61L91 64L84 67L89 70L86 73",
					"M32 28L35 22L38 29L41 23L44 28",
				].map((path, index) => (
					<g
						key={path}
						className="fl-bot-crew-motion fl-bot-chip-spark"
						data-motion-part="chip-spark"
						fill="none"
						strokeLinecap="round"
						strokeLinejoin="round"
						opacity="0"
						style={{ animationDelay: `${index * -2.4}s` }}
					>
						<path
							d={path}
							stroke={palette.body}
							strokeWidth="3.2"
							opacity=".45"
						/>
						<path d={path} stroke={palette.sclera} strokeWidth="1.5" />
					</g>
				))}
			</>
		),
	}),
	crewShape({
		id: "cog",
		label: "Cog",
		series: "tech",
		description: "A golden gear with a slowly turning rim.",
		palette: {
			body: "#dca63c",
			background: "#f8ebce",
			accent: "#b98a28",
			clothing: "#606f88",
			ink: "#51402f",
		},
		bodyMotion: "rotate",
		bodyOrigin: [50, 70],
		silhouette: radialPath(50, 70, [33, 42, 42, 33], 9),
		anchors: {
			eyes: { left: [41, 66], right: [59, 66], radius: 3.3 },
			mouth: [50, 79],
			head: { x: 50, y: 35, width: 48 },
			neck: { y: 85, bottom: 109 },
		},
		renderDetails: ({ palette }) => (
			<circle
				cx="50"
				cy="70"
				r="25"
				fill={palette.body}
				stroke={palette.accent}
				strokeWidth="2.5"
			/>
		),
	}),
	crewShape({
		id: "floppy",
		label: "Floppy",
		series: "tech",
		description: "A floppy disk with a white label and metal shutter.",
		palette: {
			body: "#30343f",
			background: "#e3e6ee",
			accent: "#c5ccd7",
			clothing: "#69728b",
			sclera: "#fffdf8",
		},
		silhouette: "M19 28H76L86 38V105H18V30Q18 28 19 28Z",
		anchors: {
			eyes: { left: [42, 74], right: [62, 74], radius: 3.5 },
			mouth: [52, 86],
			head: { x: 50, y: 30, width: 46 },
			neck: { y: 91, bottom: 108 },
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M32 29H76V49H32Z" fill={palette.clothing} />
				<g
					className="fl-bot-crew-motion fl-bot-floppy-shutter"
					data-motion-part="floppy-shutter"
				>
					<path d="M32 29H68V49H32Z" fill={palette.accent} />
					<rect x="58" y="33" width="6" height="12" fill={palette.body} />
					<path
						d="M35 31H65"
						stroke={palette.sclera}
						strokeWidth="1"
						opacity=".45"
					/>
				</g>
				<path d="M26 57H78V105H26Z" fill={palette.sclera} />
			</>
		),
	}),
];
