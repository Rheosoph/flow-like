import { BotPaintBlend } from "../bot-paints";
import type { BotShapeDefinition } from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

const fluxBody =
	"M22 51Q22 30 43 30H58Q78 30 78 51V67Q78 86 59 86H41Q22 86 22 67Z";
const lavaBody = "M41 22H59L75 61Q84 82 66 88H34Q16 82 25 61Z";
const magmaBody =
	"M20 54C20 29 34 20 51 20C74 20 82 39 81 58C80 66 77 69 75 75L74 82Q71 91 67 81L65 75Q60 77 56 76L55 90Q51 103 47 90L46 75Q42 76 36 73L34 81Q29 90 26 80L23 69Q20 64 20 54Z";

export const flowShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "flux",
		label: "Flux",
		series: "flow",
		description: "The Flow-Like icon, with an orange-to-red squircle.",
		palette: {
			body: "#ff4145",
			background: "#feebdd",
			accent: "#ff9020",
			ink: "#503327",
			cheek: "#e7bca7",
			clothing: "#485a73",
		},
		eyeStyle: "light",
		mouthColor: "sclera",
		mouthScale: 1.7,
		silhouette: fluxBody,
		anchors: {
			eyes: { left: [43, 53], right: [61, 53], radius: 3.5 },
			mouth: [51, 66],
			head: { x: 50, y: 31, width: 42 },
			neck: { y: 71, bottom: 86 },
			hand: [27, 78],
		},
		renderDetails: ({ palette, idPrefix, ...context }) => (
			<>
				<ellipse
					cx="50"
					cy="94"
					rx="20"
					ry="3.5"
					fill={palette.cheek}
					opacity="0.55"
				/>
				<BotPaintBlend
					context={{ ...context, palette, idPrefix }}
					id="flux"
					d={fluxBody}
					from="accent"
					to="body"
				/>
				<path
					d="M29 49Q30 37 39 36"
					stroke={palette.sclera}
					strokeWidth="3"
					fill="none"
					strokeLinecap="round"
					opacity="0.72"
				/>
			</>
		),
	}),
	crewShape({
		id: "lava",
		label: "Lava",
		series: "flow",
		description: "A lava lamp with floating golden bubbles.",
		palette: {
			body: "#ff2d79",
			background: "#f3e6f3",
			accent: "#ffd054",
			cheek: "#ff8569",
			clothing: "#5d4b65",
			ink: "#42303e",
		},
		eyeStyle: "round",
		mouthScale: 1.2,
		silhouette: lavaBody,
		anchors: {
			eyes: { left: [43, 55], right: [59, 55], radius: 6.8 },
			mouth: [51, 67],
			head: { x: 50, y: 23, width: 28 },
			neck: { y: 76, bottom: 89 },
			hand: [28, 80],
		},
		renderDetails: ({ palette, idPrefix, ...context }) => (
			<>
				<defs>
					<clipPath id={`${idPrefix}-lava-vessel`}>
						<path d={lavaBody} />
					</clipPath>
				</defs>
				<BotPaintBlend
					context={{ ...context, palette, idPrefix }}
					id="lava"
					d={lavaBody}
					from="cheek"
					to="body"
				/>
				{/* Bubbles move inside the glass, including at the top of their loop. */}
				<g fill={palette.accent} clipPath={`url(#${idPrefix}-lava-vessel)`}>
					<circle className="fl-bot-lava-bubble" cx="50" cy="25" r="7.5" />
					<circle
						className="fl-bot-lava-bubble"
						cx="39"
						cy="73"
						r="9"
						style={{ animationDelay: "-2s" }}
					/>
					<circle
						className="fl-bot-lava-bubble"
						cx="63"
						cy="68"
						r="6.5"
						style={{ animationDelay: "-4s" }}
					/>
					<circle
						className="fl-bot-lava-bubble"
						cx="67"
						cy="41"
						r="3.5"
						style={{ animationDelay: "-1s" }}
					/>
				</g>
				<path
					d="M33 60Q32 46 37 36"
					stroke={palette.sclera}
					strokeWidth="3"
					fill="none"
					strokeLinecap="round"
					opacity="0.65"
				/>
				<path
					d="M39 11Q39 9 41 9H59Q61 9 61 11L64 23H36Z M34 86H66L73 99H27Z"
					fill={palette.clothing}
				/>
			</>
		),
	}),
	crewShape({
		id: "magma",
		label: "Magma",
		series: "flow",
		description: "A glowing molten drop with a dripping base.",
		palette: {
			body: "#ff3846",
			background: "#ffeadf",
			accent: "#ffc23b",
			cheek: "#ffb968",
			clothing: "#634862",
		},
		cheeks: true,
		mouthScale: 1.6,
		silhouette: magmaBody,
		anchors: {
			eyes: { left: [40, 49], right: [59, 49], radius: 3.8 },
			mouth: [50, 60],
			head: { x: 50, y: 25, width: 39 },
			neck: { y: 68, bottom: 88 },
			hand: [26, 76],
		},
		renderDetails: ({ palette, idPrefix, ...context }) => (
			<>
				<ellipse
					className="fl-bot-crew-motion fl-bot-magma-puddle"
					data-motion-part="magma-puddle"
					cx="51"
					cy="95"
					rx="23"
					ry="3.5"
					fill={palette.body}
					opacity="0.3"
				/>
				<BotPaintBlend
					context={{ ...context, palette, idPrefix }}
					id="magma"
					d={magmaBody}
					from="accent"
					to="body"
					end={0.8}
				/>
				<path
					d="M27 43Q29 31 39 26"
					stroke={palette.sclera}
					strokeWidth="3"
					fill="none"
					strokeLinecap="round"
					opacity="0.65"
				/>
				<g fill={palette.accent}>
					<circle
						className="fl-bot-crew-motion fl-bot-magma-bubble"
						data-motion-part="magma-bubble"
						cx="39"
						cy="13"
						r="2.6"
					/>
					<circle
						className="fl-bot-crew-motion fl-bot-magma-bubble"
						data-motion-part="magma-bubble"
						cx="49"
						cy="6"
						r="1.3"
						style={{ animationDelay: "-1.2s" }}
					/>
					<circle
						className="fl-bot-crew-motion fl-bot-magma-bubble"
						data-motion-part="magma-bubble"
						cx="61"
						cy="10"
						r="2"
						style={{ animationDelay: "-2.4s" }}
					/>
				</g>
				<g fill={palette.body}>
					{[
						[51, 87],
						[30, 79],
						[71, 80],
					].map(([x, y], index) => (
						<ellipse
							key={`${x}-${y}`}
							className="fl-bot-crew-motion fl-bot-magma-drip"
							data-motion-part="magma-drip"
							cx={x}
							cy={y}
							rx={index ? 2.1 : 2.6}
							ry="3"
							opacity="0"
							style={{ animationDelay: `${index * -1.8}s` }}
						/>
					))}
				</g>
			</>
		),
	}),
	crewShape({
		id: "bubble",
		label: "Bubble",
		series: "flow",
		description:
			"FlowPilot's iridescent soap bubble, with a curious face and drifting reflections.",
		palette: {
			body: "#e2e0f5",
			background: "#f2edf7",
			accent: "#a386df",
			clothing: "#77cbdc",
			cheek: "#f3ad94",
			ink: "#514367",
			sclera: "#fffcff",
		},
		bodyMotion: "float",
		silhouette: ovalPath(50, 56, 33),
		anchors: {
			eyes: { left: [40, 54], right: [60, 54], radius: 3.5 },
			mouth: [50, 67],
			head: { x: 50, y: 25, width: 44 },
			neck: { y: 75, bottom: 89 },
			hand: [25, 77],
		},
		renderDetails: (context) => {
			const { palette, idPrefix } = context;
			const solid = context.solidPalette ?? palette;
			return (
				<>
					<defs>
						<linearGradient
							id={`${idPrefix}-bubble-rim`}
							x1="0"
							y1="0"
							x2="1"
							y2="1"
						>
							<stop stopColor={solid.clothing} />
							<stop offset=".4" stopColor={solid.accent} />
							<stop offset=".75" stopColor={solid.cheek} />
							<stop offset="1" stopColor={solid.clothing} />
						</linearGradient>
					</defs>
					<BotPaintBlend
						context={context}
						id="bubble-film"
						d={ovalPath(50, 56, 33)}
						from="sclera"
						to="body"
						angle={135}
						opacity={0.8}
					/>
					<circle
						cx="50"
						cy="56"
						r="32"
						fill="none"
						stroke={`url(#${idPrefix}-bubble-rim)`}
						strokeWidth="2.5"
					/>
					<BotMotion kind="spin" origin={[50, 56]} duration={24000}>
						<g fill="none" strokeLinecap="round" opacity=".55">
							<path
								d="M24 55A26 26 0 0 1 48 30"
								stroke={palette.clothing}
								strokeWidth="3"
							/>
							<path
								d="M77 53A27 27 0 0 1 51 83"
								stroke={palette.cheek}
								strokeWidth="3.5"
							/>
						</g>
					</BotMotion>
					<path
						d="M29 42Q33 34 42 32"
						stroke={palette.sclera}
						strokeWidth="4"
						fill="none"
						strokeLinecap="round"
					/>
					<circle cx="26" cy="48" r="1.7" fill={palette.sclera} />
					<BotMotion kind="rise" origin={[83, 27]} duration={6200} amount={4}>
						<circle
							cx="83"
							cy="27"
							r="4.5"
							fill={palette.sclera}
							fillOpacity=".55"
							stroke={palette.clothing}
							strokeWidth="1.2"
						/>
					</BotMotion>
					<BotMotion
						kind="rise"
						origin={[16, 81]}
						duration={7400}
						delay={-2200}
						amount={3}
					>
						<circle
							cx="16"
							cy="81"
							r="2.5"
							fill={palette.sclera}
							fillOpacity=".55"
							stroke={palette.accent}
							strokeWidth="1"
						/>
					</BotMotion>
				</>
			);
		},
	}),
];
