import { getBotClothingFit } from "../clothing-fit";
import type {
	BotEquipmentDefinition,
	BotRenderContext,
	BotShapeDefinition,
} from "../types";
import { crewShape, ovalPath } from "./helpers";
import { BotMotion } from "./motion";

export const healthcareShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "scope",
		backgroundDetailColors: ["clothing"],
		faceColor: "accent",
		label: "Scope",
		series: "healthcare",
		description:
			"A stethoscope with a round listening face and swaying earpieces.",
		palette: {
			body: "#bed4de",
			background: "#e8edf6",
			accent: "#f1f9f8",
			clothing: "#618a9b",
		},
		silhouette: ovalPath(50, 76, 25),
		anchors: {
			eyes: { left: [41, 72], right: [59, 72], radius: 3.3 },
			mouth: [50, 83],
			head: { x: 50, y: 53, width: 39 },
			neck: { y: 88, bottom: 102 },
			hand: [29, 87],
		},
		renderBackDetails: ({ palette }) => (
			<BotMotion kind="sway" origin={[50, 55]} amount={4} duration={4900}>
				<path
					d="M50 55V47Q30 48 28 32L26 20M50 47Q70 48 72 32L74 20"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="6"
					strokeLinecap="round"
				/>
				<path
					d="M28 33L26 20Q25 15 31 14M72 33L74 20Q75 15 69 14"
					fill="none"
					stroke={palette.body}
					strokeWidth="4"
					strokeLinecap="round"
				/>
				<path
					d="M30 14L34 16M70 14L66 16"
					stroke={palette.clothing}
					strokeWidth="6"
					strokeLinecap="round"
				/>
			</BotMotion>
		),
		renderDetails: ({ palette }) => (
			<>
				<circle cx="50" cy="76" r="20" fill={palette.accent} />
				<path
					d="M32 70Q34 59 45 57"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				<path
					d="M62 94Q72 88 73 79"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="2"
					strokeLinecap="round"
					opacity=".55"
				/>
			</>
		),
	}),
	crewShape({
		id: "aid",
		label: "Aid",
		series: "healthcare",
		description:
			"A teal first-aid bag with a sturdy handle and a swinging zipper pull.",
		palette: {
			body: "#78b9ac",
			background: "#e6f1e9",
			accent: "#3f807b",
			clothing: "#547f95",
			sclera: "#fffaf0",
		},
		cheeks: true,
		silhouette:
			"M18 43Q18 34 28 34H72Q82 34 82 43V89Q82 99 72 99H28Q18 99 18 89Z",
		anchors: {
			eyes: { left: [39, 65], right: [61, 65], radius: 3.6 },
			mouth: [50, 78],
			head: { x: 50, y: 35, width: 48 },
			neck: { y: 84, bottom: 100 },
			hand: [22, 86],
		},
		renderBackDetails: ({ palette, equipment }) =>
			!equipment?.head && (
				<path
					d="M37 35V26Q37 22 42 22H58Q63 22 63 26V35"
					fill="none"
					stroke={palette.accent}
					strokeWidth="6"
					strokeLinejoin="round"
				/>
			),
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M18 48H82M29 37V96M71 37V96"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2"
					opacity=".6"
				/>
				<path
					d="M46 38H54V43H59V51H54V56H46V51H41V43H46Z"
					fill={palette.sclera}
				/>
				<BotMotion
					kind="pendulum"
					origin={[78, 49]}
					amount={10}
					duration={2800}
				>
					<path d="M78 49V56" stroke={palette.sclera} strokeWidth="2" />
					<rect
						x="75.5"
						y="55"
						width="5"
						height="9"
						rx="2.5"
						fill={palette.sclera}
					/>
				</BotMotion>
				<path
					d="M33 91H67"
					stroke={palette.sclera}
					strokeWidth="2"
					strokeLinecap="round"
					opacity=".6"
				/>
			</>
		),
	}),
	crewShape({
		id: "capsule",
		label: "Capsule",
		series: "healthcare",
		description:
			"A two-tone capsule with a polished pink cap and a mint lower half.",
		palette: {
			body: "#9ccdbf",
			background: "#f4e9ef",
			accent: "#e6a7ba",
			clothing: "#9c7ba8",
			sclera: "#fff9f3",
		},
		cheeks: true,
		silhouette: "M28 46Q28 23 50 23Q72 23 72 46V79Q72 102 50 102Q28 102 28 79Z",
		anchors: {
			eyes: { left: [40, 65], right: [60, 65], radius: 3.4 },
			mouth: [50, 78],
			head: { x: 50, y: 25, width: 34 },
			neck: { y: 85, bottom: 103 },
			hand: [31, 85],
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M28 46Q28 23 50 23Q72 23 72 46V55H28Z" fill={palette.accent} />
				<path
					d="M29 55H71"
					stroke={palette.clothing}
					strokeWidth="2"
					opacity=".5"
				/>
				<BotMotion kind="sway" origin={[50, 43]} amount={5} duration={5200}>
					<path
						d="M36 45Q36 32 47 31"
						fill="none"
						stroke={palette.sclera}
						strokeWidth="4"
						strokeLinecap="round"
						opacity=".8"
					/>
				</BotMotion>
				<path
					d="M64 82Q64 93 56 95"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="2.5"
					strokeLinecap="round"
					opacity=".55"
				/>
			</>
		),
	}),
	crewShape({
		id: "molar",
		label: "Molar",
		series: "healthcare",
		description:
			"A pearly molar with two rounded roots and a twinkling enamel shine.",
		palette: {
			body: "#fffaf0",
			background: "#e5eef5",
			accent: "#b9d5df",
			clothing: "#8a9cbb",
			cheek: "#edb6ba",
		},
		cheeks: true,
		silhouette:
			"M50 36C31 23 17 34 18 51C19 66 26 72 28 90C30 108 42 106 45 87Q50 76 55 87C58 106 70 108 72 90C74 72 81 66 82 51C83 34 69 23 50 36Z",
		anchors: {
			eyes: { left: [39, 58], right: [61, 58], radius: 3.6 },
			mouth: [50, 70],
			head: { x: 50, y: 33, width: 48 },
			neck: { y: 78, bottom: 104 },
			hand: [25, 81],
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M24 50Q22 37 35 36M65 36Q74 36 76 45"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2.2"
					strokeLinecap="round"
					opacity=".7"
				/>
				<path
					d="M48 39L53 44"
					stroke={palette.accent}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				<path
					d="M30 79Q31 96 36 98M70 79Q69 96 64 98"
					fill="none"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
					opacity=".5"
				/>
				<BotMotion kind="pulse" origin={[79, 31]} amount={18} duration={3400}>
					<path
						d="M79 22L81 29L88 31L81 33L79 40L77 33L70 31L77 29Z"
						fill={palette.accent}
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "pipette",
		label: "Pipette",
		series: "healthcare",
		description:
			"A glass dropper with a soft rubber bulb and a slowly falling droplet.",
		palette: {
			body: "#d4e9ee",
			background: "#eeeaf7",
			accent: "#95c8c4",
			clothing: "#788caf",
			sclera: "#fffdf8",
		},
		silhouette: "M35 44H65V72Q65 78 59 81L54 86H46L41 81Q35 78 35 72Z",
		anchors: {
			eyes: { left: [43, 59], right: [57, 59], radius: 2.8 },
			mouth: [50, 69],
			head: { x: 50, y: 22, width: 30 },
			neck: { width: 20, y: 75, bottom: 86 },
			hand: [35, 78],
		},
		renderDetails: ({ palette, equipment }) => (
			<>
				{!equipment?.head && (
					<path
						d="M34 37V30Q34 17 50 17Q66 17 66 30V37Z"
						fill={palette.clothing}
					/>
				)}
				<rect
					x="32"
					y="36"
					width="36"
					height="11"
					rx="3"
					fill={palette.clothing}
				/>
				<path
					d="M39 39V44M46 39V44M54 39V44M61 39V44"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<path
					d="M38 73Q50 77 62 73V74Q62 78 56 81L52 84H48L44 81Q38 78 38 74Z"
					fill={palette.accent}
				/>
				<path
					d="M39 50V65M61 50V53"
					stroke={palette.sclera}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<BotMotion kind="bob" amount={2.5} duration={3000}>
					<path
						d="M50 89C49 91 46 93 46 95A4 4 0 0 0 54 95C54 93 51 91 50 89Z"
						fill={palette.accent}
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "pulse",
		label: "Pulse",
		series: "healthcare",
		description:
			"A bedside monitor with a glowing heartbeat trace above its face.",
		palette: {
			body: "#b0c7c5",
			background: "#e8eee8",
			accent: "#66a895",
			clothing: "#6e8197",
			sclera: "#f7fff8",
			ink: "#304845",
		},
		silhouette:
			"M17 40Q17 31 26 31H74Q83 31 83 40V87Q83 95 74 95H26Q17 95 17 87Z",
		anchors: {
			eyes: { left: [39, 63], right: [61, 63], radius: 3.4 },
			mouth: [50, 75],
			head: { x: 50, y: 32, width: 50 },
			neck: { y: 83, bottom: 96 },
			hand: [22, 86],
		},
		renderBackDetails: ({ palette }) => (
			<>
				<path
					d="M31 91V101M69 91V101"
					stroke={palette.clothing}
					strokeWidth="8"
					strokeLinecap="round"
				/>
				<path
					d="M83 56Q96 56 91 75Q87 88 91 93"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="3"
					strokeLinecap="round"
				/>
			</>
		),
		renderDetails: ({ palette }) => (
			<>
				<rect
					x="24"
					y="38"
					width="52"
					height="44"
					rx="5"
					fill={palette.sclera}
				/>
				<path d="M29 50H71" stroke={palette.body} strokeWidth="1" />
				<BotMotion kind="flicker" duration={2200}>
					<path
						d="M29 50H37L41 45L46 55L51 41L56 50H71"
						fill="none"
						stroke={palette.accent}
						strokeWidth="2.2"
						strokeLinejoin="round"
						strokeLinecap="round"
					/>
				</BotMotion>
				<circle cx="66" cy="88" r="2.5" fill={palette.clothing} />
				<circle cx="74" cy="88" r="2.5" fill={palette.accent} />
				<path
					d="M26 88H39"
					stroke={palette.clothing}
					strokeWidth="2"
					strokeLinecap="round"
				/>
			</>
		),
	}),
];

function MedicalTop({
	context: { anchors, palette },
	type,
}: { context: BotRenderContext; type: "coat" | "tunic" | "scrubs" }) {
	const y = 0;
	const x = 0;
	const bottom = 24;
	return (
		<g
			transform={getBotClothingFit(anchors).transform}
			stroke={palette.ink}
			strokeWidth="1.2"
			strokeLinejoin="round"
		>
			<path
				d="M-100 0H-12L0 8L12 0H100V32H-100Z"
				fill={type === "coat" ? palette.sclera : palette.clothing}
			/>
			<g>
				{type === "coat" ? (
					<>
						<path
							d={`M${x - 10} ${y}L${x} ${y + 8}L${x + 10} ${y}L${x + 5} ${bottom + 8}H${x - 5}Z`}
							fill={palette.clothing}
							stroke="none"
						/>
						<path
							d={`M${x - 14} ${y + 1}L${x - 18} ${y + 9}L${x - 9} ${y + 12}L${x - 15} ${bottom + 7}M${x + 14} ${y + 1}L${x + 18} ${y + 9}L${x + 9} ${y + 12}L${x + 15} ${bottom + 7}`}
							fill="none"
							stroke={palette.clothing}
						/>
						<path
							d={`M${x - 24} ${y + 3}V${y + 10}Q${x - 24} ${y + 16} ${x - 19} ${y + 16}Q${x - 14} ${y + 16} ${x - 14} ${y + 10}V${y + 5}M${x + 21} ${y + 3}V${y + 9}`}
							fill="none"
							strokeWidth="1.8"
						/>
						<circle cx={x + 21} cy={y + 12} r="3" fill={palette.accent} />
					</>
				) : type === "tunic" ? (
					<>
						<path
							d={`M${x - 12} ${y}L${x} ${y + 6}L${x - 7} ${y + 11}L${x - 16} ${y + 3}ZM${x + 12} ${y}L${x} ${y + 6}L${x + 7} ${y + 11}L${x + 16} ${y + 3}Z`}
							fill={palette.sclera}
							stroke="none"
						/>
						<path
							d={`M${x} ${y + 9}V${bottom + 8}`}
							fill="none"
							opacity=".45"
						/>
						<rect
							x={x + 17}
							y={y + 8}
							width="11"
							height="8"
							rx="1.5"
							fill={palette.sclera}
							stroke="none"
						/>
						<path
							d={`M${x + 20} ${y + 11}H${x + 25}M${x + 20} ${y + 14}H${x + 23}`}
							stroke={palette.clothing}
							strokeWidth="1.1"
						/>
					</>
				) : (
					<>
						<path
							d={`M${x - 13} ${y + 1}L${x} ${y + 11}L${x + 13} ${y + 1}`}
							fill="none"
							stroke={palette.accent}
							strokeWidth="3"
						/>
						<path
							d={`M${x + 17} ${y + 9}H${x + 29}V${y + 18}Q${x + 23} ${y + 22} ${x + 17} ${y + 18}Z`}
							fill="none"
							stroke={palette.sclera}
							opacity=".7"
						/>
					</>
				)}
			</g>
		</g>
	);
}

export const healthcareEquipment: readonly BotEquipmentDefinition[] = [
	{
		id: "doctor-coat",
		label: "Doctor's coat",
		slot: "neck",
		render: (context) => <MedicalTop context={context} type="coat" />,
	},
	{
		id: "nurse-tunic",
		label: "Nurse's tunic",
		slot: "neck",
		render: (context) => <MedicalTop context={context} type="tunic" />,
	},
	{
		id: "scrub-top",
		label: "Scrub top",
		slot: "neck",
		render: (context) => <MedicalTop context={context} type="scrubs" />,
	},
	{
		id: "scrub-cap",
		label: "Scrub cap",
		slot: "head",
		render: ({ anchors, palette }) => (
			<g
				transform={`translate(${anchors.head.x} ${anchors.head.y}) scale(${anchors.head.width / 44})`}
				stroke={palette.ink}
				strokeWidth="1.4"
				strokeLinejoin="round"
			>
				<BotMotion kind="flutter" origin={[21, 3]} amount={7} duration={4200}>
					<path
						d="M20 1Q32 -4 32 1L23 5L31 12L27 16L19 5Z"
						fill={palette.clothing}
					/>
				</BotMotion>
				<path
					d="M-23 5L-22 -7Q-20 -15 -10 -14Q0 -21 10 -14Q20 -16 22 -7L23 5Q0 1 -23 5Z"
					fill={palette.clothing}
				/>
				<path d="M-23 2Q0 -2 23 2V7Q0 3 -23 7Z" fill={palette.accent} />
				<path
					d="M-15 -10L-16 -5M15 -10L17 -5"
					stroke={palette.sclera}
					strokeWidth="2"
					strokeLinecap="round"
					opacity=".6"
				/>
			</g>
		),
	},
];
