import { getBotClothingFit } from "../clothing-fit";
import type { BotEquipmentDefinition, BotShapeDefinition } from "../types";
import { crewShape } from "./helpers";
import { BotMotion } from "./motion";

export const workplaceShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "brief",
		label: "Brief",
		series: "workplace",
		description:
			"A caramel briefcase with brass clasps and a swinging luggage tag.",
		palette: {
			body: "#c58b62",
			background: "#eee5da",
			accent: "#f2cc77",
			clothing: "#775949",
		},
		silhouette:
			"M17 42Q17 36 24 36H76Q83 36 83 42V91Q83 99 75 99H25Q17 99 17 91Z",
		anchors: {
			eyes: { left: [39, 67], right: [61, 67], radius: 3.5 },
			mouth: [50, 80],
			head: { x: 50, y: 34, width: 47 },
			neck: { y: 86, bottom: 100 },
			hand: [22, 83],
		},
		renderBackDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<path
					d="M37 38V29Q37 24 43 24H57Q63 24 63 29V38"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="6"
					strokeLinecap="round"
				/>
			),
		renderDetails: ({ palette, equipment }) => (
			<>
				<path d="M18 42Q50 51 82 42V53Q50 61 18 53Z" fill={palette.clothing} />
				<path
					d="M23 59V90Q23 93 27 93H73Q77 93 77 90V59"
					fill="none"
					stroke={palette.accent}
					strokeWidth="1.2"
					strokeDasharray="2 3"
					opacity=".65"
				/>
				<rect
					x="29"
					y="47"
					width="7"
					height="12"
					rx="2"
					fill={palette.accent}
				/>
				<rect
					x="64"
					y="47"
					width="7"
					height="12"
					rx="2"
					fill={palette.accent}
				/>
				{!equipment?.hand && (
					<BotMotion kind="sway" origin={[22, 54]} amount={7} duration={4200}>
						<path d="M22 54L16 69" stroke={palette.clothing} strokeWidth="2" />
						<path d="M12 67H23L26 72V85H10V72Z" fill={palette.sclera} />
						<circle cx="18" cy="72" r="1.7" fill={palette.clothing} />
						<path
							d="M14 78H22M14 81H19"
							stroke={palette.clothing}
							strokeWidth="1.2"
						/>
					</BotMotion>
				)}
			</>
		),
	}),
	crewShape({
		id: "dial",
		label: "Dial",
		series: "workplace",
		description:
			"A reception telephone whose curved handset gently rings above the keypad.",
		palette: {
			body: "#d68c95",
			background: "#f7e6eb",
			accent: "#f2bec0",
			clothing: "#925e78",
		},
		silhouette:
			"M30 42H70Q77 42 79 50L88 87Q90 97 79 99H21Q10 97 12 87L21 50Q23 42 30 42Z",
		anchors: {
			eyes: { left: [39, 60], right: [61, 60], radius: 3.4 },
			mouth: [50, 73],
			head: { x: 50, y: 32, width: 49 },
			neck: { y: 84, bottom: 100 },
			hand: [23, 85],
		},
		renderBackDetails: ({ palette }) => (
			<path
				d="M81 42Q94 46 89 52Q82 55 90 59Q97 63 89 67Q82 71 90 75Q96 80 88 84"
				fill="none"
				stroke={palette.clothing}
				strokeWidth="2.5"
				strokeLinecap="round"
			/>
		),
		renderDetails: ({ palette, equipment }) => (
			<>
				<path
					d="M17 87Q50 94 83 87L85 94Q50 102 15 94Z"
					fill={palette.clothing}
					opacity=".6"
				/>
				<path
					d="M28 48H72"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				{!equipment?.head && (
					<BotMotion kind="sway" origin={[50, 35]} amount={3} duration={2000}>
						<path
							d="M18 35Q19 23 32 24L37 25Q40 26 39 33L37 39Q35 42 29 40H23Q17 40 18 35Z M82 35Q81 23 68 24L63 25Q60 26 61 33L63 39Q65 42 71 40H77Q83 40 82 35Z"
							fill={palette.clothing}
						/>
						<path
							d="M31 28Q50 21 69 28"
							fill="none"
							stroke={palette.clothing}
							strokeWidth="9"
							strokeLinecap="round"
						/>
						<path
							d="M35 26Q50 22 65 26"
							fill="none"
							stroke={palette.accent}
							strokeWidth="2"
							strokeLinecap="round"
						/>
					</BotMotion>
				)}
				{[80, 85].map((y) => (
					<g key={y} fill={palette.sclera}>
						{[40, 50, 60].map((x) => (
							<rect key={x} x={x - 3} y={y} width="6" height="3" rx="1.2" />
						))}
					</g>
				))}
			</>
		),
	}),
	crewShape({
		id: "calc",
		label: "Calc",
		series: "workplace",
		description:
			"A mint pocket calculator with a friendly display and softly blinking keys.",
		palette: {
			body: "#9cbda9",
			background: "#e6eee3",
			accent: "#e7b978",
			clothing: "#57776f",
			ink: "#374a43",
			sclera: "#eef2da",
		},
		silhouette:
			"M27 27H73Q79 27 79 34V94Q79 101 72 101H28Q21 101 21 94V34Q21 27 27 27Z",
		anchors: {
			eyes: { left: [40, 51], right: [60, 51], radius: 3.1 },
			mouth: [50, 62],
			head: { x: 50, y: 27, width: 42 },
			neck: { y: 80, bottom: 103 },
			hand: [23, 85],
		},
		renderDetails: ({ palette }) => (
			<>
				<rect
					x="27"
					y="35"
					width="46"
					height="35"
					rx="4"
					fill={palette.clothing}
				/>
				<rect
					x="30"
					y="38"
					width="40"
					height="29"
					rx="2"
					fill={palette.sclera}
				/>
				<path
					d="M31 30H49"
					stroke={palette.clothing}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				{[77, 86, 95].map((y) => (
					<g key={y} fill={palette.clothing}>
						{[31, 43, 55].map((x) => (
							<rect key={x} x={x - 3} y={y - 3} width="8" height="6" rx="1.5" />
						))}
					</g>
				))}
				<rect
					x="65"
					y="74"
					width="8"
					height="6"
					rx="1.5"
					fill={palette.accent}
				/>
				<BotMotion kind="flicker" duration={3600}>
					<rect
						x="65"
						y="83"
						width="8"
						height="15"
						rx="2"
						fill={palette.accent}
					/>
					<path d="M67 89H71M67 93H71" stroke={palette.ink} strokeWidth="1.3" />
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "type",
		label: "Type",
		series: "workplace",
		description:
			"A compact keyboard with rounded keycaps and a bouncing spacebar.",
		palette: {
			body: "#a9b8ce",
			background: "#e7edf7",
			accent: "#e9b77c",
			clothing: "#6c819f",
			sclera: "#f2f3f5",
		},
		silhouette:
			"M17 41H83Q88 41 89 47L94 88Q95 97 86 99H14Q5 97 6 88L11 47Q12 41 17 41Z",
		anchors: {
			eyes: { left: [39, 63], right: [61, 63], radius: 3.5 },
			mouth: [50, 76],
			head: { x: 50, y: 41, width: 55 },
			neck: { y: 86, bottom: 100 },
			hand: [18, 84],
		},
		renderBackDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<path
					d="M50 41V31Q50 25 58 25H66Q72 25 72 18"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="3"
					strokeLinecap="round"
				/>
			),
		renderDetails: ({ palette }) => (
			<>
				<path d="M8 90Q50 96 92 90V96Q50 102 8 96Z" fill={palette.clothing} />
				{[22, 36, 50, 64, 78].map((x) => (
					<rect
						key={x}
						x={x - 5}
						y="46"
						width="10"
						height="7"
						rx="2"
						fill={x === 22 ? palette.accent : palette.sclera}
					/>
				))}
				{[60, 72, 84].map((y) => (
					<g key={y} fill={palette.sclera}>
						<rect x="15" y={y} width="9" height="7" rx="2" />
						<rect x="76" y={y} width="9" height="7" rx="2" />
					</g>
				))}
				<BotMotion kind="bob" amount={0.8} duration={2300}>
					<rect
						x="30"
						y="84"
						width="40"
						height="7"
						rx="2"
						fill={palette.accent}
					/>
					<path
						d="M35 86H65"
						stroke={palette.sclera}
						strokeWidth="1.2"
						strokeLinecap="round"
						opacity=".6"
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "badge",
		label: "Badge",
		series: "workplace",
		description:
			"A violet ID card with a woven lanyard and a little clipped corner.",
		palette: {
			body: "#aaa0ca",
			background: "#eee8f7",
			accent: "#efbc76",
			clothing: "#756c9c",
			sclera: "#faf5ec",
		},
		cheeks: true,
		silhouette:
			"M28 38H72Q78 38 78 44V91L69 101H28Q22 101 22 95V44Q22 38 28 38Z",
		anchors: {
			eyes: { left: [40, 63], right: [60, 63], radius: 3.3 },
			mouth: [50, 76],
			head: { x: 50, y: 38, width: 40 },
			neck: { y: 84, bottom: 102 },
			hand: [24, 86],
		},
		renderBackDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<BotMotion kind="sway" origin={[50, 39]} amount={3} duration={5200}>
					<path
						d="M47 39L35 22Q30 13 40 12H60Q70 13 65 22L53 39"
						fill="none"
						stroke={palette.clothing}
						strokeWidth="5"
						strokeLinejoin="round"
					/>
					<path
						d="M37 18L48 35M63 18L52 35"
						fill="none"
						stroke={palette.accent}
						strokeWidth="1.2"
						strokeDasharray="2 2"
					/>
				</BotMotion>
			),
		renderDetails: ({ palette }) => (
			<>
				<path d="M28 50H72V94H28Z" fill={palette.sclera} />
				<rect
					x="43"
					y="35"
					width="14"
					height="10"
					rx="3"
					fill={palette.clothing}
				/>
				<path
					d="M47 40H53"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<path
					d="M35 86H65M41 90H59"
					stroke={palette.clothing}
					strokeWidth="1.8"
					strokeLinecap="round"
				/>
				<path d="M69 101V91H78Z" fill={palette.clothing} />
			</>
		),
	}),
	crewShape({
		id: "agenda",
		label: "Agenda",
		series: "workplace",
		description:
			"A standing desk calendar with copper rings and a fluttering page corner.",
		palette: {
			body: "#f6e7cd",
			background: "#e5eee7",
			accent: "#e6a26b",
			clothing: "#799992",
		},
		silhouette: "M24 32H76L84 99H16Z",
		anchors: {
			eyes: { left: [40, 61], right: [60, 61], radius: 3.4 },
			mouth: [50, 74],
			head: { x: 50, y: 31, width: 44 },
			neck: { y: 84, bottom: 100 },
			hand: [23, 85],
		},
		renderBackDetails: ({ palette }) => (
			<path d="M24 32H76L90 101H10Z" fill={palette.clothing} />
		),
		renderDetails: ({ palette, equipment }) => (
			<>
				<path d="M24 32H76L78 47H22Z" fill={palette.accent} />
				{!equipment?.head &&
					[35, 50, 65].map((x) => (
						<path
							key={x}
							d={`M${x - 2} 36V27Q${x - 2} 23 ${x + 2} 23Q${x + 6} 23 ${x + 6} 27V33`}
							fill="none"
							stroke={palette.clothing}
							strokeWidth="2.5"
							strokeLinecap="round"
						/>
					))}
				<path
					d="M28 83H72M32 88H66M36 93H60"
					stroke={palette.clothing}
					strokeWidth="1.2"
					strokeDasharray="4 3"
					opacity=".65"
				/>
				<path d="M67 99V85H82Z" fill={palette.accent} />
				<BotMotion kind="flutter" origin={[67, 85]} amount={4} duration={4700}>
					<path d="M67 99V85H82Z" fill={palette.sclera} />
					<path
						d="M67 99V85H82"
						fill="none"
						stroke={palette.accent}
						strokeWidth="1.2"
					/>
				</BotMotion>
			</>
		),
	}),
];

export const workplaceEquipment: readonly BotEquipmentDefinition[] = [
	{
		id: "office-blazer",
		label: "Office blazer",
		slot: "neck",
		render: ({ anchors, palette }) => {
			const y = 0;
			const x = 0;
			return (
				<g transform={getBotClothingFit(anchors).transform}>
					<g stroke={palette.ink} strokeWidth="1.2" strokeLinejoin="round">
						<path d="M-100 0H100V32H-100Z" fill={palette.clothing} />
						<path
							d={`M${x - 13} ${y}H${x + 13}L${x} ${y + 25}Z`}
							fill={palette.sclera}
						/>
						<path
							d={`M${x - 14} ${y}L${x - 18} ${y + 6}L${x - 11} ${y + 8}L${x - 15} ${y + 13}L${x} ${y + 25}L${x + 15} ${y + 13}L${x + 11} ${y + 8}L${x + 18} ${y + 6}L${x + 14} ${y}`}
							fill="none"
						/>
						<BotMotion
							kind="sway"
							origin={[x, y + 2]}
							amount={3}
							duration={5000}
						>
							<path
								d={`M${x - 3} ${y + 1}H${x + 3}L${x + 2} ${y + 5}L${x + 4} ${y + 15}L${x} ${y + 19}L${x - 4} ${y + 15}L${x - 2} ${y + 5}Z`}
								fill={palette.accent}
								stroke="none"
							/>
						</BotMotion>
					</g>
				</g>
			);
		},
	},
	{
		id: "reception-vest",
		label: "Reception vest",
		slot: "neck",
		render: ({ anchors, palette }) => {
			const y = 0;
			const x = 0;
			return (
				<g transform={getBotClothingFit(anchors).transform}>
					<g stroke={palette.ink} strokeWidth="1.1" strokeLinejoin="round">
						<path d="M-100 0H100V32H-100Z" fill={palette.sclera} />
						<path
							d={`M${x - 25} ${y}H${x - 11}L${x} ${y + 11}L${x + 11} ${y}H${x + 25}L${x + 29} ${y + 28}H${x - 29}Z`}
							fill={palette.clothing}
						/>
						<path
							d={`M${x - 11} ${y}L${x} ${y + 4}L${x + 11} ${y}L${x + 6} ${y + 9}L${x} ${y + 5}L${x - 6} ${y + 9}Z`}
							fill={palette.sclera}
						/>
						<path d={`M${x} ${y + 11}V${y + 28}`} />
						<rect
							x={x + 9}
							y={y + 10}
							width="11"
							height="5"
							rx="1"
							fill={palette.accent}
							stroke="none"
						/>
						<circle
							cx={x - 3}
							cy={y + 15}
							r="1.1"
							fill={palette.accent}
							stroke="none"
						/>
						<circle
							cx={x - 3}
							cy={y + 21}
							r="1.1"
							fill={palette.accent}
							stroke="none"
						/>
					</g>
				</g>
			);
		},
	},
	{
		id: "office-lanyard",
		label: "Office lanyard",
		slot: "neck",
		render: ({ anchors, palette }) => {
			const y = 0;
			const x = 0;
			const drop = 8;
			return (
				<g transform={getBotClothingFit(anchors).transform}>
					<path
						d={`M${x - 13} ${y}L${x} ${y + drop}L${x + 13} ${y}`}
						fill="none"
						stroke={palette.clothing}
						strokeWidth="3.2"
						strokeLinejoin="round"
					/>
					<BotMotion
						kind="sway"
						origin={[x, y + drop]}
						amount={4}
						duration={4500}
					>
						<g transform={`translate(${x} ${y + drop})`}>
							<rect
								x="-7"
								y="0"
								width="14"
								height="14"
								rx="2"
								fill={palette.sclera}
								stroke={palette.ink}
								strokeWidth="1"
							/>
							<path d="M-5 3H5" stroke={palette.accent} strokeWidth="2" />
							<circle cx="-2" cy="8" r="2" fill={palette.clothing} />
							<path
								d="M1 7H4M1 10H4"
								stroke={palette.clothing}
								strokeWidth="1"
							/>
						</g>
					</BotMotion>
				</g>
			);
		},
	},
	{
		id: "reception-headset",
		label: "Reception headset",
		slot: "head",
		render: ({ anchors, palette }) => {
			const x = anchors.head.x;
			const y = (anchors.eyes.left[1] + anchors.eyes.right[1]) / 2;
			const half = Math.max(
				anchors.head.width / 2,
				(anchors.eyes.right[0] - anchors.eyes.left[0]) / 2 +
					anchors.eyes.radius +
					9,
			);
			return (
				<g strokeLinecap="round" strokeLinejoin="round">
					<path
						d={`M${x - half} ${y}V${anchors.head.y + 8}Q${x - half} ${anchors.head.y - 9} ${x} ${anchors.head.y - 9}Q${x + half} ${anchors.head.y - 9} ${x + half} ${anchors.head.y + 8}V${y}`}
						fill="none"
						stroke={palette.clothing}
						strokeWidth="4"
					/>
					<rect
						x={x - half - 4}
						y={y - 5}
						width="8"
						height="14"
						rx="4"
						fill={palette.clothing}
						stroke={palette.ink}
						strokeWidth="1.3"
					/>
					<rect
						x={x + half - 4}
						y={y - 5}
						width="8"
						height="14"
						rx="4"
						fill={palette.clothing}
						stroke={palette.ink}
						strokeWidth="1.3"
					/>
					<BotMotion
						kind="sway"
						origin={[x + half, y + 5]}
						amount={3}
						duration={4900}
					>
						<path
							d={`M${x + half} ${y + 5}Q${x + half + 2} ${anchors.mouth[1] + 7} ${anchors.mouth[0] + 12} ${anchors.mouth[1] + 5}`}
							fill="none"
							stroke={palette.ink}
							strokeWidth="2"
						/>
						<rect
							x={anchors.mouth[0] + 8}
							y={anchors.mouth[1] + 2}
							width="7"
							height="5"
							rx="2.5"
							fill={palette.accent}
						/>
					</BotMotion>
				</g>
			);
		},
	},
];
