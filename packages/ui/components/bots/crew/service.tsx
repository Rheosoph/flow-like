import type { ReactNode } from "react";
import type {
	BotEquipmentDefinition,
	BotRenderContext,
	BotShapeDefinition,
} from "../types";
import { crewShape } from "./helpers";
import { BotMotion } from "./motion";

export const serviceShapes: readonly BotShapeDefinition[] = [
	crewShape({
		id: "radio",
		label: "Radio",
		series: "service",
		description:
			"A police radio with a springy antenna and a glowing signal light.",
		palette: {
			body: "#55789b",
			background: "#e4ecf7",
			clothing: "#3c526e",
			accent: "#efc975",
			sclera: "#d8ebe8",
		},
		silhouette:
			"M24 42Q24 35 31 35H69Q76 35 76 42V95Q76 101 69 101H31Q24 101 24 95Z",
		anchors: {
			eyes: { left: [40, 62], right: [60, 62], radius: 3.5 },
			mouth: [50, 73],
			head: { x: 50, y: 36, width: 43 },
			neck: { y: 82, bottom: 102 },
			hand: [26, 87],
		},
		renderBackDetails: ({ palette }) => (
			<BotMotion kind="sway" origin={[32, 37]} duration={4600} amount={4}>
				<path
					d="M32 37V12"
					stroke={palette.clothing}
					strokeWidth="5"
					strokeLinecap="round"
				/>
				<path
					d="M32 13V19"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
			</BotMotion>
		),
		renderDetails: ({ palette }) => (
			<>
				<rect
					x="57"
					y="29"
					width="12"
					height="8"
					rx="2"
					fill={palette.clothing}
				/>
				<rect
					x="29"
					y="48"
					width="42"
					height="32"
					rx="7"
					fill={palette.sclera}
				/>
				<path
					d="M33 89H67M33 94H67"
					stroke={palette.clothing}
					strokeWidth="2.5"
					strokeLinecap="round"
				/>
				<BotMotion kind="pulse" origin={[65, 42]} duration={3200} amount={12}>
					<circle cx="65" cy="42" r="2.7" fill={palette.accent} />
				</BotMotion>
				<path
					d="M19 50V61M19 67V74"
					stroke={palette.clothing}
					strokeWidth="4"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "hydrant",
		label: "Hydrant",
		series: "service",
		description:
			"A cherry-red fire hydrant with brass outlets and a dangling cap chain.",
		palette: {
			body: "#d67b66",
			background: "#fae8db",
			clothing: "#ae594c",
			accent: "#edc568",
			sclera: "#ffeed5",
		},
		silhouette:
			"M29 45Q29 27 50 27Q71 27 71 45V53H82V75H71V94H78V102H22V94H29V75H18V53H29Z",
		anchors: {
			eyes: { left: [40, 63], right: [60, 63], radius: 3.4 },
			mouth: [50, 75],
			head: { x: 50, y: 28, width: 37 },
			neck: { y: 84, bottom: 103 },
			hand: [25, 87],
		},
		renderDetails: ({ palette }) => (
			<>
				<path d="M46 27V22H54V27" fill={palette.accent} />
				<path d="M27 44H73V49H27ZM26 92H74V97H26Z" fill={palette.clothing} />
				<path
					d="M33 39Q37 32 45 32"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
					opacity=".6"
				/>
				<rect
					x="13"
					y="52"
					width="12"
					height="24"
					rx="3"
					fill={palette.accent}
				/>
				<rect
					x="75"
					y="52"
					width="12"
					height="24"
					rx="3"
					fill={palette.accent}
				/>
				<path
					d="M18 58V70M82 58V70"
					stroke={palette.clothing}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<BotMotion kind="pendulum" origin={[79, 74]} duration={4400} amount={6}>
					<path
						d="M79 74Q83 88 70 89"
						fill="none"
						stroke={palette.accent}
						strokeWidth="1.7"
						strokeDasharray="2 2"
						strokeLinecap="round"
					/>
				</BotMotion>
				<path
					d="M41 85H59"
					stroke={palette.accent}
					strokeWidth="3"
					strokeLinecap="round"
				/>
			</>
		),
	}),
	crewShape({
		id: "rotor",
		label: "Rotor",
		series: "service",
		description:
			"A rescue helicopter with a roomy cockpit, spinning tail rotor, and landing skids.",
		palette: {
			body: "#e8b55c",
			background: "#e2eef5",
			clothing: "#627e99",
			accent: "#ca6b54",
			sclera: "#e4f3ed",
		},
		bodyMotion: "float",
		silhouette:
			"M29 61Q32 43 57 43Q85 43 88 68Q90 84 70 89H44Q31 87 28 76L12 69L9 48H16L21 61Z",
		anchors: {
			eyes: { left: [48, 64], right: [68, 64], radius: 3.5 },
			mouth: [58, 76],
			head: { x: 57, y: 44, width: 42 },
			neck: { y: 81, bottom: 90 },
			hand: [32, 80],
		},
		renderBackDetails: ({ palette }) => (
			<>
				<path
					d="M43 84V95M73 84V95M33 95H82Q87 95 88 90"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<path d="M56 43V33" stroke={palette.clothing} strokeWidth="4" />
				<BotMotion kind="flutter" origin={[56, 32]} duration={1500} amount={4}>
					<path
						d="M24 32H88"
						stroke={palette.clothing}
						strokeWidth="3.5"
						strokeLinecap="round"
					/>
					<path
						d="M27 32H35M77 32H85"
						stroke={palette.sclera}
						strokeWidth="2"
					/>
				</BotMotion>
			</>
		),
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M37 60Q41 49 58 49Q77 49 81 63L79 77Q58 84 36 74Z"
					fill={palette.sclera}
				/>
				<path
					d="M45 52L40 57"
					stroke={palette.body}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<path
					d="M37 84H72"
					stroke={palette.accent}
					strokeWidth="4"
					strokeLinecap="round"
				/>
				<BotMotion kind="spin" origin={[13, 55]} duration={3200}>
					<path
						d="M13 47V63M5 55H21"
						stroke={palette.clothing}
						strokeWidth="3"
						strokeLinecap="round"
					/>
				</BotMotion>
				<circle cx="13" cy="55" r="2.4" fill={palette.accent} />
			</>
		),
	}),
	crewShape({
		id: "rucksack",
		label: "Rucksack",
		series: "service",
		description:
			"An olive field pack with rolled bedding, utility pockets, and a swinging tag.",
		palette: {
			body: "#829577",
			background: "#e8ecdf",
			clothing: "#566b56",
			accent: "#d7c98a",
			sclera: "#f6efd8",
		},
		silhouette:
			"M24 49Q24 32 50 32Q76 32 76 49V95Q76 102 69 102H31Q24 102 24 95Z",
		anchors: {
			eyes: { left: [39, 64], right: [61, 64], radius: 3.7 },
			mouth: [50, 76],
			head: { x: 50, y: 33, width: 42 },
			neck: { y: 84, bottom: 103 },
			hand: [24, 87],
		},
		renderBackDetails: ({ palette }) => (
			<>
				<path
					d="M40 34V27Q50 20 60 27V34"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="4"
				/>
				<rect
					x="14"
					y="69"
					width="14"
					height="27"
					rx="4"
					fill={palette.clothing}
				/>
				<rect
					x="72"
					y="69"
					width="14"
					height="27"
					rx="4"
					fill={palette.clothing}
				/>
			</>
		),
		renderDetails: ({ palette }) => (
			<>
				<rect
					x="23"
					y="30"
					width="54"
					height="17"
					rx="8.5"
					fill={palette.accent}
				/>
				<path
					d="M35 31V46M65 31V46"
					stroke={palette.clothing}
					strokeWidth="4"
				/>
				<path
					d="M27 49Q50 57 73 49"
					fill="none"
					stroke={palette.clothing}
					strokeWidth="2"
				/>
				<path d="M31 85H69V96Q50 101 31 96Z" fill={palette.clothing} />
				<path
					d="M34 87H66M19 75H23M77 75H81"
					stroke={palette.accent}
					strokeWidth="2"
					strokeLinecap="round"
				/>
				<BotMotion kind="pendulum" origin={[64, 87]} duration={4200} amount={8}>
					<rect
						x="61.5"
						y="88"
						width="5"
						height="7"
						rx="1.5"
						fill={palette.accent}
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "pylon",
		label: "Pylon",
		series: "service",
		description:
			"A bright traffic cone with broad reflective stripes and a sturdy square foot.",
		palette: {
			body: "#ed9a60",
			background: "#f6eadc",
			clothing: "#747e8d",
			accent: "#c97347",
			sclera: "#fff4dd",
		},
		silhouette: "M44 29Q50 25 56 29L76 92H84V101H16V92H24Z",
		anchors: {
			eyes: { left: [42, 67], right: [58, 67], radius: 3.1 },
			mouth: [50, 78],
			head: { x: 50, y: 28, width: 24 },
			neck: { y: 85, bottom: 102 },
			hand: [29, 88],
		},
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M40 42H60L64 54H36ZM29 78H71L75 90H25Z"
					fill={palette.sclera}
				/>
				<path d="M16 92H84V101H16Z" fill={palette.clothing} />
				<path
					d="M22 96H78"
					stroke={palette.sclera}
					strokeWidth="2"
					opacity=".45"
				/>
				<path
					d="M46 32L42 43"
					stroke={palette.sclera}
					strokeWidth="2.2"
					strokeLinecap="round"
					opacity=".6"
				/>
				<BotMotion kind="flicker" origin={[28, 52]} duration={4900} amount={5}>
					<path
						d="M23 43L20 38M76 43L79 38"
						stroke={palette.accent}
						strokeWidth="2.2"
						strokeLinecap="round"
					/>
				</BotMotion>
			</>
		),
	}),
	crewShape({
		id: "buoy",
		label: "Buoy",
		series: "service",
		description:
			"A rescue ring bobbing on the water, with coral bands and a looped safety rope.",
		palette: {
			body: "#fae8c8",
			background: "#dfedf3",
			clothing: "#65899b",
			accent: "#dd8669",
			sclera: "#fff8e6",
		},
		bodyMotion: "float",
		silhouette:
			"M16 65a34 34 0 1 0 68 0a34 34 0 1 0 -68 0ZM50 38a16 16 0 1 1 0 32a16 16 0 1 1 0 -32Z",
		anchors: {
			eyes: { left: [39, 76], right: [61, 76], radius: 3.2 },
			mouth: [50, 86],
			head: { x: 50, y: 32, width: 43 },
			neck: { y: 92, bottom: 100 },
			hand: [23, 83],
		},
		renderBackDetails: ({ palette }) => (
			<path
				d="M25 38Q8 40 12 63Q4 85 28 95M75 38Q92 40 88 63Q96 85 72 95"
				fill="none"
				stroke={palette.clothing}
				strokeWidth="2"
				strokeLinecap="round"
			/>
		),
		renderDetails: ({ palette }) => (
			<>
				<path
					d="M38 33Q50 29 62 33L58 39Q50 35 42 39ZM18 53L33 54V66L16 68Q15 60 18 53ZM82 53L67 54V66L84 68Q85 60 82 53Z"
					fill={palette.accent}
				/>
				<path
					d="M30 40Q24 43 22 48"
					fill="none"
					stroke={palette.sclera}
					strokeWidth="3"
					strokeLinecap="round"
				/>
				<path
					d="M42 97Q50 99 58 97"
					fill="none"
					stroke={palette.accent}
					strokeWidth="4"
				/>
			</>
		),
	}),
];

function ServiceHead({
	context: { anchors, palette },
	children,
}: { context: BotRenderContext; children: ReactNode }) {
	const scaleX = anchors.head.width / 44;
	const scaleY = Math.min(scaleX, Math.max(0.2, (anchors.head.y - 3) / 25));
	return (
		<g
			transform={`translate(${anchors.head.x} ${anchors.head.y}) scale(${scaleX} ${scaleY})`}
			stroke={palette.ink}
			strokeWidth="1.3"
			strokeLinejoin="round"
			strokeLinecap="round"
		>
			{children}
		</g>
	);
}

function ServiceUniform({
	context: { anchors, palette },
	variant,
}: {
	context: BotRenderContext;
	variant: "police" | "fire" | "field" | "rescue";
}) {
	const y = anchors.neck.y;
	const center = anchors.head.x;
	const height = 24;
	const bottom = y + height;
	const scaleY = Math.max(1, anchors.neck.bottom - y) / height;
	return (
		<g
			transform={`translate(0 ${y}) scale(1 ${scaleY}) translate(0 ${-y})`}
			stroke={palette.ink}
			strokeWidth="1.2"
			strokeLinejoin="round"
		>
			<path
				d={`M0 ${y}H${center - 11}L${center} ${y + 6}L${center + 11} ${y}H100V${bottom + 8}H0Z`}
				fill={variant === "rescue" ? palette.accent : palette.clothing}
			/>
			{variant === "police" && (
				<>
					<path
						d={`M${center - 12} ${y}L${center} ${y + 6}L${center - 7} ${y + 12}L${center - 18} ${y}ZM${center + 12} ${y}L${center} ${y + 6}L${center + 7} ${y + 12}L${center + 18} ${y}Z`}
						fill={palette.sclera}
					/>
					<path
						d={`M${center - 2} ${y + 7}H${center + 2}L${center + 4} ${bottom - 1}L${center} ${bottom + 3}L${center - 4} ${bottom - 1}Z`}
						fill={palette.ink}
					/>
					<path
						d={`M${center + 17} ${y + 5}h8v5q0 4 -4 6q-4 -2 -4 -6Z`}
						fill={palette.accent}
					/>
				</>
			)}
			{variant === "fire" && (
				<>
					<path
						d={`M0 ${y + height * 0.4}H100M0 ${y + height * 0.85}H100`}
						stroke={palette.accent}
						strokeWidth="4"
					/>
					<path
						d={`M${center} ${y + 6}V${bottom + 5}`}
						stroke={palette.sclera}
						strokeWidth="2"
					/>
					<path
						d={`M${center - 23} ${y + 2}V${bottom}M${center + 23} ${y + 2}V${bottom}`}
						stroke={palette.accent}
						strokeWidth="3"
					/>
				</>
			)}
			{variant === "field" && (
				<>
					<path
						d={`M${center - 28} ${y}h12l7 7l-12 5l-10 -4ZM${center + 17} ${y + 2}h13l5 8l-16 3ZM${center - 8} ${bottom - 8}l10 -5l12 8l-7 9l-13 -4Z`}
						fill={palette.accent}
						stroke="none"
						opacity=".65"
					/>
					<path
						d={`M${center - 15} ${y + 5}v${height - 3}M${center + 15} ${y + 5}v${height - 3}`}
						stroke={palette.ink}
						strokeWidth="3"
						opacity=".35"
					/>
					<path
						d={`M${center - 6} ${y + 5}Q${center} ${y + 16} ${center + 6} ${y + 5}`}
						fill="none"
						stroke={palette.sclera}
					/>
					<rect
						x={center - 2.5}
						y={y + 10}
						width="5"
						height="7"
						rx="1.5"
						fill={palette.sclera}
					/>
				</>
			)}
			{variant === "rescue" && (
				<>
					<path
						d={`M${center - 6} ${y + 3}L${center - 3} ${bottom + 6}H${center + 3}L${center + 6} ${y + 3}`}
						fill={palette.clothing}
					/>
					<path
						d={`M${center - 23} ${y + 5}V${y + height * 0.6}M${center + 23} ${y + 5}V${y + height * 0.6}`}
						stroke={palette.sclera}
						strokeWidth="5"
					/>
					<path
						d={`M0 ${y + height * 0.75}H100`}
						stroke={palette.clothing}
						strokeWidth="4"
					/>
					<rect
						x={center - 4}
						y={y + height * 0.75 - 3}
						width="8"
						height="6"
						rx="1"
						fill={palette.sclera}
					/>
				</>
			)}
		</g>
	);
}

export const serviceEquipment: readonly BotEquipmentDefinition[] = [
	{
		id: "police-cap",
		label: "Police cap",
		slot: "head",
		render: (context) => (
			<ServiceHead context={context}>
				<path
					d="M-22 -1L-25 -12Q0 -27 25 -12L22 -1Z"
					fill={context.palette.clothing}
				/>
				<path d="M-22 -3H22V1H-22Z" fill={context.palette.accent} />
				<path d="M-20 1H20Q16 9 0 8Q-16 9 -20 1Z" fill={context.palette.ink} />
				<path
					d="M-5 -16H5V-10Q5 -5 0 -3Q-5 -5 -5 -10Z"
					fill={context.palette.accent}
				/>
			</ServiceHead>
		),
	},
	{
		id: "police-uniform",
		label: "Police uniform",
		slot: "neck",
		render: (context) => <ServiceUniform context={context} variant="police" />,
	},
	{
		id: "fire-helmet",
		label: "Fire helmet",
		slot: "head",
		render: (context) => (
			<ServiceHead context={context}>
				<path
					d="M-23 0V-5Q-23 -25 0 -25Q23 -25 23 -5V0Z"
					fill={context.palette.clothing}
				/>
				<path d="M-4 -23H4V0H-4Z" fill={context.palette.accent} />
				<path
					d="M-29 0Q0 -7 29 0L27 6Q0 1 -27 6Z"
					fill={context.palette.clothing}
				/>
				<path d="M-5 -15H5V-9L0 -5L-5 -9Z" fill={context.palette.sclera} />
			</ServiceHead>
		),
	},
	{
		id: "fire-coat",
		label: "Firefighter coat",
		slot: "neck",
		render: (context) => <ServiceUniform context={context} variant="fire" />,
	},
	{
		id: "field-cap",
		label: "Field cap",
		slot: "head",
		render: (context) => (
			<ServiceHead context={context}>
				<path d="M-22 0V-13Q0 -24 22 -13V0Z" fill={context.palette.clothing} />
				<path
					d="M-19 -14L-9 -17L-3 -11L-10 -7L-21 -9ZM9 -17L20 -13L22 -5L11 -7Z"
					fill={context.palette.accent}
					stroke="none"
				/>
				<path
					d="M-22 -2H23L30 4Q13 10 -1 3H-22Z"
					fill={context.palette.clothing}
				/>
				<path d="M-2 -15H3V-10H-2Z" fill={context.palette.sclera} />
			</ServiceHead>
		),
	},
	{
		id: "field-jacket",
		label: "Field jacket",
		slot: "neck",
		render: (context) => <ServiceUniform context={context} variant="field" />,
	},
	{
		id: "rescue-vest",
		label: "Rescue vest",
		slot: "neck",
		render: (context) => <ServiceUniform context={context} variant="rescue" />,
	},
];
