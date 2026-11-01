import { BotPaintBlend } from "../bot-paints";
import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";

const slateBody =
	"M30 30L66 27Q77 26 80 38L87 77Q91 92 76 96L36 102Q20 105 17 89L11 50Q8 34 30 30Z";

export const proShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "atlas",
		label: "Atlas",
		series: "pro",
		description: "A calm blue sphere with a soft reflection.",
		palette: {
			body: "#2f607b",
			background: "#e2ebef",
			ink: "#e2eff4",
			clothing: "#254255",
			accent: "#83bad3",
		},
		silhouette: ovalPath(50, 69, 34),
		eyeStyle: "light",
		mouthStyle: "none",
		anchors: {
			eyes: { left: [41, 65], right: [59, 65], radius: 2.5 },
			head: { x: 50, y: 35, width: 46 },
		},
		renderDetails: ({ palette }) => (
			<path
				d="M28 57Q31 43 44 40"
				fill="none"
				stroke={palette.accent}
				strokeWidth="3"
				opacity=".5"
				strokeLinecap="round"
			/>
		),
	}),
	crewShape({
		id: "prism",
		label: "Prism",
		series: "pro",
		description: "A warm stone prism with a shaded facet.",
		palette: {
			body: "#cabea5",
			background: "#eee8df",
			accent: "#a99a7f",
			clothing: "#59656e",
		},
		silhouette: "M47 33Q50 27 53 33L84 92Q87 100 50 100Q13 100 16 92Z",
		mouthStyle: "line",
		anchors: {
			head: { x: 50, y: 32, width: 26 },
			eyes: { left: [42, 70], right: [58, 70], radius: 3.2 },
			mouth: [50, 82],
			neck: { y: 84, bottom: 100 },
		},
		renderDetails: ({ palette }) => (
			<path
				d="M50 30L84 92Q87 100 50 100Z"
				fill={palette.accent}
				opacity=".6"
			/>
		),
	}),
	crewShape({
		id: "slate",
		label: "Slate",
		series: "pro",
		description:
			"A polished charcoal stone with cool highlights and a soft smile.",
		palette: {
			body: "#444d5e",
			background: "#e5e9ee",
			ink: "#f5f4ee",
			accent: "#9aacbf",
			clothing: "#26374c",
		},
		silhouette: slateBody,
		eyeStyle: "light",
		mouthColor: "sclera",
		mouthScale: 0.9,
		anchors: {
			eyes: { left: [39, 61], right: [61, 61], radius: 3.2 },
			mouth: [50, 75],
			head: { x: 49, y: 30, width: 44 },
			neck: { y: 83, bottom: 102 },
			hand: [25, 85],
		},
		renderDetails: ({ palette, idPrefix, ...context }) => (
			<>
				<defs>
					<clipPath id={`${idPrefix}-slate-stone`}>
						<path d={slateBody} />
					</clipPath>
				</defs>
				<g
					data-feature="slate-stone"
					clipPath={`url(#${idPrefix}-slate-stone)`}
				>
					<BotPaintBlend
						context={{ ...context, palette, idPrefix }}
						id="slate"
						d={slateBody}
						from="accent"
						to="body"
						angle={45}
						end={0.55}
						opacity={0.42}
					/>
					<path
						data-feature="slate-bevel"
						d="M69 28Q78 28 80 38L87 77Q91 92 76 96L36 102Q23 105 18 92L67 89Q79 86 77 75Z"
						fill={palette.clothing}
						opacity=".5"
					/>
					<path
						d="M29 32L65 29Q72 28 75 34L30 39Q21 41 22 52L17 73L13 50Q10 36 29 32Z"
						fill={palette.accent}
						opacity=".3"
					/>
					<path
						d="M26 46Q36 39 52 38"
						fill="none"
						stroke={palette.accent}
						strokeWidth="1.4"
						strokeLinecap="round"
						opacity=".65"
					/>
					<path
						d="M74 78Q77 87 66 90"
						fill="none"
						stroke={palette.accent}
						strokeWidth="1.2"
						strokeLinecap="round"
						opacity=".25"
					/>
					<circle cx="26" cy="75" r="1" fill={palette.accent} opacity=".4" />
					<circle cx="29" cy="79" r=".65" fill={palette.accent} opacity=".3" />
				</g>
			</>
		),
	}),
	crewShape({
		id: "facet",
		label: "Facet",
		series: "pro",
		description: "A polished green gem with fine-cut edges.",
		palette: {
			body: "#2e725d",
			background: "#dceee8",
			ink: "#e7f6ec",
			accent: "#75af98",
			clothing: "#234d43",
		},
		silhouette: "M29 30H71L89 52L69 96H31L11 52Z",
		eyeStyle: "light",
		anchors: {
			head: { x: 50, y: 30, width: 42 },
			eyes: { left: [40, 67], right: [60, 67], radius: 2.8 },
			mouth: [50, 77],
		},
		renderDetails: ({ palette }) => (
			<g fill="none" stroke={palette.accent} strokeWidth="1.3" opacity=".6">
				<path d="M14 52H86M29 31L39 52L32 92M71 31L61 52L68 92M39 52L31 32H69L61 52" />
			</g>
		),
	}),
	crewShape({
		id: "otto",
		label: "Otto",
		series: "pro",
		description: "A wise owl with cream eye patches and expressive brows.",
		palette: {
			body: "#82796b",
			background: "#ebe8e3",
			ink: "#302e35",
			accent: "#e9b94e",
			clothing: "#4a5550",
		},
		silhouette:
			"M24 48L21 27L36 36Q50 31 64 36L79 27L76 48Q87 63 82 88Q69 101 50 101Q31 101 18 88Q13 63 24 48Z",
		eyeStyle: "round",
		mouthStyle: "beak",
		anchors: {
			head: { x: 50, y: 31, width: 52 },
			eyes: { left: [38, 64], right: [62, 64], radius: 8.7 },
			mouth: [50, 78],
		},
		renderDetails: ({ palette }) => (
			<>
				<ellipse
					cx="36"
					cy="66"
					rx="13"
					ry="15"
					fill={palette.sclera}
					opacity=".85"
				/>
				<ellipse
					cx="64"
					cy="66"
					rx="13"
					ry="15"
					fill={palette.sclera}
					opacity=".85"
				/>
			</>
		),
		renderFaceDetails: ({ palette }) => (
			<g
				fill="none"
				stroke={palette.ink}
				strokeWidth="2.4"
				strokeLinecap="round"
			>
				<path d="M27 49Q36 46 43 50M57 50Q64 46 73 49" />
			</g>
		),
	}),
];
