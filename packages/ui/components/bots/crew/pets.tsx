import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

const hamiBody = `${ovalPath(27, 43, 8)}${ovalPath(73, 43, 8)}${ovalPath(50, 77, 40, 35)}`;

export const petsShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "miso",
		label: "Miso",
		series: "pets",
		description: "Cat with pointed ears and whiskers.",
		palette: {
			body: "#5db5a7",
			background: "#dcf0eb",
			cheek: "#efb5aa",
			ink: "#293d3d",
			clothing: "#47647c",
			accent: "#edc888",
		},
		anchors: { head: { x: 50, y: 36, width: 51 }, mouth: [50, 79] },
		mouthStyle: "cat",
		silhouette:
			"M15 69Q15 51 21 43L23 18L40 33Q50 30 60 33L77 18L80 43Q85 53 85 69V97Q50 111 15 97Z",
		renderDetails: ({ palette }) => (
			<g fill={palette.cheek}>
				<path d="M25 25L36 35L24 41Z" />
				<path d="M75 25L76 41L64 35Z" />
			</g>
		),
		renderFaceDetails: ({ palette }) => (
			<g stroke={palette.ink} strokeWidth="1.6" strokeLinecap="round">
				<path d="M47 73Q50 71 53 73L50 76Z" fill={palette.ink} stroke="none" />
				<path
					d="M31 75L20 73M30 79H19M31 83L20 86M69 75L80 73M70 79H81M69 83L80 86"
					fill="none"
				/>
			</g>
		),
	}),
	crewShape({
		id: "bun",
		label: "Bun",
		series: "pets",
		description: "Rabbit with long ears and pink cheeks.",
		palette: {
			body: "#fffdf6",
			background: "#e8d5c4",
			cheek: "#f1b8b1",
			ink: "#38313b",
			clothing: "#a3758d",
			accent: "#e4c4a1",
		},
		anchors: {
			eyes: { left: [39, 70], right: [61, 70], radius: 3.8 },
			mouth: [50, 83],
			head: { x: 50, y: 45, width: 48 },
			neck: { y: 86, bottom: 102 },
		},
		mouthStyle: "cat",
		cheeks: true,
		silhouette: "M12 91C12 62 29 43 50 43S88 62 88 91V103H12Z",
		renderBackDetails: ({ palette }) => (
			<>
				<BotMotion kind="flutter" origin={[39, 48]} duration={6200} amount={4}>
					<path
						d="M35 49C29 35 29 9 36 8C44 6 46 33 43 49Z"
						fill={palette.body}
					/>
					<ellipse
						cx="36.5"
						cy="27"
						rx="2.7"
						ry="14"
						transform="rotate(-3 36.5 27)"
						fill={palette.cheek}
					/>
				</BotMotion>
				<BotMotion
					kind="flutter"
					origin={[61, 48]}
					duration={6200}
					delay={-3100}
					amount={4}
				>
					<path
						d="M57 49C54 32 56 7 63 8C72 9 71 35 65 49Z"
						fill={palette.body}
					/>
					<ellipse
						cx="63"
						cy="27"
						rx="2.7"
						ry="14"
						transform="rotate(3 63 27)"
						fill={palette.cheek}
					/>
				</BotMotion>
			</>
		),
		renderFaceDetails: ({ palette }) => (
			<path d="M47.5 77Q50 75 52.5 77L50 80Z" fill={palette.ink} />
		),
	}),
	crewShape({
		id: "kuma",
		label: "Kuma",
		series: "pets",
		description: "Brown bear with round ears and a cream muzzle.",
		palette: {
			body: "#a77c54",
			background: "#eee0cf",
			accent: "#e5c5a2",
			ink: "#342e35",
			clothing: "#516a78",
			cheek: "#c78e75",
		},
		anchors: {
			mouth: [50, 81],
			head: { x: 50, y: 38, width: 51 },
			neck: { y: 89, bottom: 100 },
		},
		mouthStyle: "cat",
		silhouette: `${ovalPath(24, 42, 11)}${ovalPath(76, 42, 11)}${ovalPath(50, 75, 38, 36)}`,
		renderDetails: ({ palette }) => (
			<g fill={palette.accent}>
				<circle cx="24" cy="42" r="6.5" />
				<circle cx="76" cy="42" r="6.5" />
				<ellipse cx="50" cy="80" rx="12.5" ry="10" />
			</g>
		),
		renderFaceDetails: ({ palette }) => (
			<path
				d="M46.5 74Q50 72 53.5 74Q54 77 50 78Q46 77 46.5 74Z"
				fill={palette.ink}
			/>
		),
	}),
	crewShape({
		id: "fynn",
		label: "Fynn",
		series: "pets",
		description: "Fox with tall ears and white cheeks.",
		palette: {
			body: "#d9703a",
			background: "#fbe6d6",
			sclera: "#fff7ec",
			accent: "#f4cbb1",
			ink: "#352c30",
			clothing: "#55766b",
		},
		anchors: {
			mouth: [50, 82],
			head: { x: 50, y: 34, width: 52 },
			neck: { y: 87, bottom: 100 },
		},
		mouthStyle: "cat",
		silhouette:
			"M13 67Q13 51 20 42L18 15L39 32Q50 29 61 32L82 15L80 42Q87 51 87 67V104H13Z",
		renderDetails: ({ palette }) => (
			<>
				<g fill={palette.accent}>
					<path d="M23 23L35 35L24 41Z" />
					<path d="M77 23L76 41L65 35Z" />
				</g>
				<path
					d="M13 76Q31 67 50 87Q69 67 87 76V104H13Z"
					fill={palette.sclera}
				/>
			</>
		),
		renderFaceDetails: ({ palette }) => (
			<path d="M46 76Q50 74 54 76L50 81Z" fill={palette.ink} />
		),
	}),
	crewShape({
		id: "hami",
		label: "Hami",
		series: "pets",
		description: "Hamster with full cheeks and little round ears.",
		palette: {
			body: "#e3ad70",
			background: "#fcead8",
			cheek: "#efb0a2",
			sclera: "#fff9eb",
			ink: "#3a3036",
			clothing: "#6d89a1",
			accent: "#f5cb8d",
		},
		anchors: { mouth: [50, 78], head: { x: 50, y: 41, width: 49 } },
		mouthStyle: "cat",
		silhouette: hamiBody,
		renderDetails: ({ palette, idPrefix }) => (
			<>
				<defs>
					<clipPath id={`${idPrefix}-hami-coat`}>
						<path d={hamiBody} />
					</clipPath>
				</defs>
				<g fill={palette.cheek}>
					<circle cx="27" cy="43" r="4.8" />
					<circle cx="73" cy="43" r="4.8" />
				</g>
				<path
					d="M13 87C19 69 37 69 50 83C63 69 81 69 87 87L88 107H12Z"
					fill={palette.sclera}
					clipPath={`url(#${idPrefix}-hami-coat)`}
				/>
				<g fill={palette.cheek}>
					<ellipse cx="28" cy="83" rx="5" ry="2.6" />
					<ellipse cx="72" cy="83" rx="5" ry="2.6" />
				</g>
			</>
		),
		renderFaceDetails: ({ palette }) => (
			<path d="M47.5 72Q50 70 52.5 72L50 75Z" fill={palette.ink} />
		),
	}),
];
