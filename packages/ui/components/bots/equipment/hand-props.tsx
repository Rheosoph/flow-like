import type { ReactNode } from "react";
import type { BotEquipmentDefinition, BotPalette } from "../types";

function handProp(
	id: string,
	label: string,
	artwork: (palette: BotPalette) => ReactNode,
	compactSkill?: string,
): BotEquipmentDefinition {
	return {
		id,
		label,
		slot: "hand",
		...(compactSkill ? { compactSkill } : {}),
		render: ({ anchors, palette }) => (
			<g
				transform={`translate(${anchors.hand.join(" ")}) rotate(-9)`}
				stroke={palette.ink}
				strokeWidth="1.5"
				strokeLinejoin="round"
				strokeLinecap="round"
				fill="none"
			>
				{artwork(palette)}
			</g>
		),
	};
}

export const handPropsBySeries: Readonly<
	Record<string, readonly BotEquipmentDefinition[]>
> = {
	flow: [
		handProp(
			"workflow-cube",
			"Workflow cube",
			(p) => (
				<>
					<path d="M0 -21L14 -13V4L0 12L-14 4V-13Z" fill={p.clothing} />
					<path d="M-14 -13L0 -5L14 -13M0 -5V12" stroke={p.sclera} />
					<path
						d="M-8 -10V-2L-3 1M5 -8V0L10 -3"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<circle cx="0" cy="-14" r="3" fill={p.accent} />
					<circle cx="-7" cy="2" r="2" fill={p.sclera} />
				</>
			),
			"automation",
		),
		handProp(
			"connector-key",
			"Connector key",
			(p) => (
				<>
					<path d="M-5 -21H5V-10H-5Z" fill={p.sclera} />
					<path d="M-3 -18V-14M3 -18V-14" stroke={p.clothing} strokeWidth="2" />
					<rect
						x="-8"
						y="-10"
						width="16"
						height="22"
						rx="5"
						fill={p.clothing}
					/>
					<path
						d="M0 6V-5M0 -2L-4 -5M0 2L4 -1"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<circle cx="0" cy="9" r="1" fill={p.sclera} stroke="none" />
				</>
			),
			"device",
		),
		handProp("launch-rocket", "Launch rocket", (p) => (
			<>
				<path d="M-6 1L-12 8V-4L-6 -10M6 1L12 8V-4L6 -10" fill={p.clothing} />
				<path d="M-6 3V-10Q-5 -17 0 -22Q5 -17 6 -10V3Z" fill={p.sclera} />
				<circle cx="0" cy="-9" r="3.5" fill={p.accent} />
				<path d="M-4 4L0 12L4 4M-6 0H6" fill={p.accent} />
			</>
		)),
		handProp("flow-pennant", "Flow pennant", (p) => (
			<>
				<path d="M-10 13V-20" strokeWidth="2.5" />
				<path d="M-9 -19H14L7 -11L14 -3H-9Z" fill={p.clothing} />
				<path d="M-3 -11H5M1 -15V-7" stroke={p.sclera} />
				<circle cx="-10" cy="-21" r="2" fill={p.accent} />
			</>
		)),
		handProp(
			"patch-cable",
			"Patch cable",
			(p) => (
				<>
					<path
						d="M-8 -8V3C-8 17 11 17 11 3V-6"
						stroke={p.clothing}
						strokeWidth="4"
					/>
					<rect x="-12" y="-17" width="8" height="11" rx="2" fill={p.accent} />
					<rect x="7" y="-15" width="8" height="11" rx="2" fill={p.accent} />
					<path d="M-10 -21V-17M-6 -21V-17M9 -19V-15M13 -19V-15" />
				</>
			),
			"automation",
		),
	],
	basics: [
		handProp("shopping-tote", "Shopping tote", (p) => (
			<>
				<path
					d="M-7 -10V-17Q0 -24 7 -17V-10"
					stroke={p.clothing}
					strokeWidth="3"
				/>
				<path d="M-13 -10H13L11 12H-11Z" fill={p.sclera} />
				<path d="M-7 -10V-3M7 -10V-3" stroke={p.clothing} strokeWidth="2.5" />
				<path d="M0 -2Q-7 -7 -7 0Q-6 4 0 7Q6 4 7 0Q7 -7 0 -2Z" fill={p.cheek} />
			</>
		)),
		handProp("travel-thermos", "Travel thermos", (p) => (
			<>
				<rect x="-8" y="-17" width="16" height="29" rx="5" fill={p.clothing} />
				<path d="M-8 -13H8V-20H-8ZM-8 6H8V10H-8Z" fill={p.accent} />
				<path d="M8 -8H12Q16 -1 9 4" strokeWidth="2.5" />
				<path d="M-4 -7V1" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		handProp(
			"pocket-camera",
			"Pocket camera",
			(p) => (
				<>
					<path d="M-6 -13L-3 -18H6L9 -13" fill={p.accent} />
					<rect
						x="-15"
						y="-13"
						width="30"
						height="23"
						rx="4"
						fill={p.clothing}
					/>
					<circle cx="2" cy="-2" r="8" fill={p.sclera} />
					<circle cx="2" cy="-2" r="5" fill={p.accent} />
					<path d="M0 -4L3 -6M-11 -8H-7" stroke={p.sclera} strokeWidth="2" />
					<path d="M-14 5H-9" stroke={p.accent} />
				</>
			),
			"image",
		),
		handProp("rain-umbrella", "Umbrella", (p) => (
			<>
				<path d="M0 -19V7Q0 15 -6 12Q-9 10 -7 7" strokeWidth="2.5" />
				<path
					d="M-16 -5Q-14 -21 0 -21Q14 -21 16 -5Q12 -9 8 -5Q4 -9 0 -5Q-4 -9 -8 -5Q-12 -9 -16 -5Z"
					fill={p.clothing}
				/>
				<path d="M0 -21Q-7 -16 -8 -5M0 -21Q7 -16 8 -5" stroke={p.accent} />
			</>
		)),
		handProp("pocket-book", "Paperback", (p) => (
			<>
				<path
					d="M-12 -16H12V11H-9Q-13 11 -13 7V-12Q-13 -16 -9 -16Z"
					fill={p.clothing}
				/>
				<path d="M-9 -16V5M-9 5H12V11H-9Q-13 8 -9 5Z" fill={p.sclera} />
				<path d="M-4 -9H7M-4 -5H4M1 -16V-9L4 -11L7 -9V-16" stroke={p.accent} />
				<path d="M-7 8H9" strokeWidth=".8" />
			</>
		)),
		handProp("water-bottle", "Water bottle", (p) => (
			<>
				<path
					d="M-5 -14V-9Q-10 -7 -10 -2V10Q-10 13 -7 13H7Q10 13 10 10V-2Q10 -7 5 -9V-14"
					fill={p.clothing}
				/>
				<rect x="-6" y="-20" width="12" height="7" rx="2" fill={p.accent} />
				<path d="M-10 -3H10V7H-10Z" fill={p.sclera} />
				<path d="M0 -1Q-7 6 0 6Q7 6 0 -1Z" fill={p.accent} />
			</>
		)),
	],
	nature: [
		handProp("pressed-leaf-frame", "Pressed leaf frame", (p) => (
			<>
				<rect x="-12" y="-21" width="24" height="32" rx="2" fill={p.accent} />
				<path d="M-8 -17H8V7H-8Z" fill={p.sclera} />
				<path d="M-4 2Q-10 -12 5 -14Q11 -1 -4 2Z" fill={p.clothing} />
				<path d="M-5 5L4 -11M-1 -3L-5 -7M1 -6L5 -5" stroke={p.accent} />
			</>
		)),
		handProp("butterfly-kite", "Butterfly kite", (p) => (
			<>
				<path
					d="M0 -9Q-16 -28 -15 -12Q-15 -4 -5 -5Q-15 2 -5 5L0 -3L5 5Q15 2 5 -5Q15 -4 15 -12Q16 -28 0 -9Z"
					fill={p.cheek}
				/>
				<path
					d="M0 -14V-3M-10 -14L-4 -9M10 -14L4 -9"
					stroke={p.clothing}
					strokeWidth="2"
				/>
				<path d="M0 -3Q-6 3 2 6T-1 13" />
				<path d="M-3 8L1 10L-3 12M1 10L5 8V12Z" fill={p.accent} />
			</>
		)),
		handProp("pinecone", "Pinecone", (p) => (
			<>
				<path
					d="M0 -21Q-10 -17 -12 -6Q-15 5 0 12Q15 5 12 -6Q10 -17 0 -21Z"
					fill={p.accent}
				/>
				<path
					d="M-6 -14L0 -9L6 -14M-10 -7L-5 -2L0 -7L5 -2L10 -7M-11 1L-6 6L0 1L6 6L11 1M-4 -15L0 -19L4 -15"
					fill={p.clothing}
				/>
				<path d="M0 6V10" stroke={p.sclera} />
			</>
		)),
		handProp("leaf-sprig", "Leaf sprig", (p) => (
			<>
				<path d="M-2 13Q3 -1 2 -17" strokeWidth="2" />
				<path
					d="M1 -4Q-14 -1 -12 -15Q1 -15 1 -4ZM3 -11Q3 -23 14 -21Q16 -9 3 -11ZM0 6Q4 -7 14 -2Q13 10 0 6Z"
					fill={p.accent}
				/>
				<path d="M-9 -11L0 -4M10 -17L4 -12M10 1L2 5" stroke={p.clothing} />
			</>
		)),
		handProp("acorn-keepsake", "Acorn", (p) => (
			<>
				<path d="M-10 -5Q-12 8 0 14Q12 8 10 -5Z" fill={p.accent} />
				<path d="M-13 -5Q-13 -17 0 -17Q13 -17 13 -5Z" fill={p.clothing} />
				<path d="M0 -17Q-3 -23 3 -22M-8 -10L-4 -6M-3 -12L1 -8M3 -13L7 -9M7 -7L10 -5" />
				<path d="M-5 0Q-5 6 -1 8" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
	],
	pets: [
		handProp("squeaky-bone", "Squeaky bone", (p) => (
			<>
				<path
					d="M-6 -12Q-15 -19 -15 -10Q-20 -3 -9 -2L3 5Q2 15 10 10Q19 13 15 4Q19 -4 8 -2Z"
					fill={p.sclera}
				/>
				<path d="M-5 -7L6 1" stroke={p.accent} strokeWidth="4" />
				<path d="M-10 -12L-8 -10M11 5L13 7" stroke={p.cheek} />
			</>
		)),
		handProp("pet-grooming-brush", "Grooming brush", (p) => (
			<>
				<rect x="-3" y="-2" width="6" height="14" rx="3" fill={p.accent} />
				<rect x="-11" y="-20" width="22" height="19" rx="7" fill={p.clothing} />
				<path
					d="M-6 -16V-6M0 -17V-5M6 -16V-6M-8 -12H8"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<circle cx="0" cy="8" r="1" fill={p.clothing} />
			</>
		)),
		handProp("feather-teaser", "Feather teaser", (p) => (
			<>
				<path
					d="M-8 11L-1 -18Q14 -23 11 -8"
					stroke={p.clothing}
					strokeWidth="2.5"
				/>
				<path
					d="M10 -8Q-2 -11 3 2L10 8Q18 1 15 -5Q14 -9 10 -8Z"
					fill={p.cheek}
				/>
				<path d="M10 -6V8M6 -2L10 1L14 -2M7 3L10 5L14 2" stroke={p.sclera} />
				<path d="M-8 12L-6 5" stroke={p.accent} strokeWidth="4" />
			</>
		)),
		handProp("yarn-ball", "Yarn ball", (p) => (
			<>
				<circle cx="-1" cy="-3" r="12" fill={p.cheek} />
				<path d="M-11 -9Q0 -6 10 4M-13 -3Q-3 1 6 8M-8 -13Q5 -10 11 -1M-8 7Q-4 -5 5 -13M-2 9Q0 -1 9 -9M11 1Q18 1 15 10Q13 15 7 12" />
			</>
		)),
		handProp("pet-treat-jar", "Treat jar", (p) => (
			<>
				<rect x="-11" y="-12" width="22" height="25" rx="5" fill={p.sclera} />
				<rect x="-12" y="-18" width="24" height="7" rx="2" fill={p.clothing} />
				<path
					d="M-4 0Q-10 -6 -10 0Q-10 6 -4 3H4Q10 9 10 3Q10 -3 4 0Z"
					fill={p.accent}
				/>
			</>
		)),
	],
	zoo: [
		handProp("keeper-bucket", "Keeper bucket", (p) => (
			<>
				<path
					d="M-11 -9Q-14 -25 0 -25Q14 -25 11 -9"
					stroke={p.clothing}
					strokeWidth="2"
				/>
				<path d="M-14 -11H14L10 11H-10Z" fill={p.clothing} />
				<ellipse cx="0" cy="-11" rx="14" ry="4" fill={p.accent} />
				<path d="M-7 -7L-5 6M7 -7L5 6" stroke={p.sclera} />
				<path d="M-4 -13L-2 -20L2 -17L7 -22L9 -14" fill={p.cheek} />
			</>
		)),
		handProp("animal-track-cast", "Animal track cast", (p) => (
			<>
				<path
					d="M-13 -14L-5 -20L7 -18L14 -8L11 7L0 12L-11 7L-15 -4Z"
					fill={p.sclera}
				/>
				<path
					d="M-7 2Q-6 -8 0 -7Q6 -8 7 2Q6 7 0 4Q-6 7 -7 2Z"
					fill={p.clothing}
				/>
				<ellipse cx="-8" cy="-8" rx="2.5" ry="3" fill={p.accent} />
				<ellipse cx="-3" cy="-13" rx="2.5" ry="3" fill={p.accent} />
				<ellipse cx="3" cy="-13" rx="2.5" ry="3" fill={p.accent} />
				<ellipse cx="8" cy="-8" rx="2.5" ry="3" fill={p.accent} />
			</>
		)),
		handProp("nesting-box", "Nesting box", (p) => (
			<>
				<path d="M-11 -9H11V11H-11Z" fill={p.accent} />
				<path d="M-15 -8L0 -22L15 -8L12 -5L0 -16L-12 -5Z" fill={p.clothing} />
				<circle cx="0" cy="-4" r="4" fill={p.ink} />
				<path d="M-3 3H3M-7 8H7" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		handProp(
			"field-binoculars",
			"Binoculars",
			(p) => (
				<>
					<path d="M-12 -13H-5L-3 5H-16ZM5 -13H12L16 5H3Z" fill={p.clothing} />
					<path d="M-4 -7H4V0H-4Z" fill={p.accent} />
					<ellipse cx="-9" cy="5" rx="7" ry="6" fill={p.sclera} />
					<ellipse cx="9" cy="5" rx="7" ry="6" fill={p.sclera} />
					<path d="M-12 5L-9 2M6 5L9 2" stroke={p.accent} strokeWidth="2" />
				</>
			),
			"search",
		),
		handProp("bamboo-stalk", "Bamboo stalk", (p) => (
			<>
				<path d="M-4 -20H3V13H-4Z" fill={p.accent} />
				<path
					d="M-5 -11H4M-5 0H4M-5 10H4"
					stroke={p.clothing}
					strokeWidth="2"
				/>
				<path
					d="M3 -9Q4 -22 14 -19Q14 -9 3 -9ZM-4 2Q-16 -1 -13 -12Q-2 -8 -4 2ZM3 4Q10 -7 16 -1Q12 7 3 4Z"
					fill={p.clothing}
				/>
			</>
		)),
	],
	tech: [
		handProp(
			"soldering-iron",
			"Soldering iron",
			(p) => (
				<>
					<path d="M-10 10L1 -7L7 -3L-4 14Z" fill={p.clothing} />
					<path d="M1 -7L9 -20L12 -18L7 -3Z" fill={p.sclera} />
					<path d="M9 -20L13 -23M-8 13Q-17 8 -12 2" />
					<path d="M-6 8L0 -1" stroke={p.accent} strokeWidth="2" />
					<path d="M16 -17Q13 -21 17 -24" stroke={p.cheek} />
				</>
			),
			"tools",
		),
		handProp(
			"retro-floppy",
			"Floppy disk",
			(p) => (
				<>
					<path d="M-14 -19H9L14 -14V12H-14Z" fill={p.clothing} />
					<path d="M-8 -19H8V-7H-8Z" fill={p.sclera} />
					<path d="M3 -17H6V-9H3Z" fill={p.accent} />
					<path d="M-9 12V-1H9V12" fill={p.accent} />
					<path d="M-5 3H5M-5 7H3" stroke={p.sclera} />
				</>
			),
			"files",
		),
		handProp(
			"smartphone",
			"Smartphone",
			(p) => (
				<>
					<rect
						x="-10"
						y="-22"
						width="20"
						height="35"
						rx="4"
						fill={p.clothing}
					/>
					<rect x="-7" y="-17" width="14" height="22" rx="1" fill={p.sclera} />
					<path d="M-4 -12H4V-6H0L-3 -3V-6H-4Z" fill={p.accent} />
					<path d="M-2 -19H2M-2 9H2" stroke={p.sclera} strokeWidth="2" />
				</>
			),
			"chat",
		),
		handProp(
			"drawing-tablet",
			"Drawing tablet",
			(p) => (
				<>
					<rect
						x="-13"
						y="-18"
						width="26"
						height="32"
						rx="4"
						fill={p.clothing}
					/>
					<rect x="-9" y="-13" width="18" height="21" rx="1" fill={p.sclera} />
					<path d="M-6 3L-1 -4L3 0L6 -6" stroke={p.accent} strokeWidth="2" />
					<circle cx="0" cy="11" r="1" fill={p.accent} stroke="none" />
					<path d="M13 7L17 -13" stroke={p.accent} strokeWidth="3" />
				</>
			),
			"image",
		),
		handProp(
			"circuit-board",
			"Circuit board",
			(p) => (
				<>
					<rect
						x="-13"
						y="-17"
						width="26"
						height="29"
						rx="2"
						fill={p.clothing}
					/>
					<path
						d="M-11 -10H-7V-5H7V-12H11M-11 5H-7V0H7V7H11M0 -17V-8M0 4V12"
						stroke={p.accent}
					/>
					<rect x="-5" y="-7" width="10" height="10" rx="1" fill={p.ink} />
					<path d="M-2 -4L2 0M2 -4L-2 0" stroke={p.sclera} />
					<circle cx="-9" cy="-13" r="1" fill={p.sclera} stroke="none" />
				</>
			),
			"code",
		),
		handProp(
			"camera-drone",
			"Camera drone",
			(p) => (
				<>
					<path
						d="M-5 -2L-11 -10M5 -2L11 -10M-5 3L-11 10M5 3L11 10"
						strokeWidth="3"
					/>
					<ellipse cx="-11" cy="-11" rx="7" ry="3" fill={p.sclera} />
					<ellipse cx="11" cy="-11" rx="7" ry="3" fill={p.sclera} />
					<ellipse cx="-11" cy="10" rx="7" ry="3" fill={p.sclera} />
					<ellipse cx="11" cy="10" rx="7" ry="3" fill={p.sclera} />
					<rect x="-6" y="-7" width="12" height="15" rx="4" fill={p.clothing} />
					<circle cx="0" cy="1" r="3" fill={p.accent} />
				</>
			),
			"video",
		),
	],
	snacks: [
		handProp("soft-pretzel", "Soft pretzel", (p) => (
			<>
				<path
					d="M0 8C-30 7 -14 -27 -4 -13L8 6M0 8C30 7 14 -27 4 -13L-8 6"
					stroke={p.ink}
					strokeWidth="9"
				/>
				<path
					d="M0 8C-30 7 -14 -27 -4 -13L8 6M0 8C30 7 14 -27 4 -13L-8 6"
					stroke={p.accent}
					strokeWidth="6"
				/>
				<path
					d="M-11 -9L-9 -8M9 -9L11 -8M-4 6L-2 7M3 -5L5 -4"
					stroke={p.sclera}
					strokeWidth="2"
				/>
			</>
		)),
		handProp("ramen-bowl", "Ramen bowl", (p) => (
			<>
				<path d="M-14 -5H14Q12 8 0 10Q-12 8 -14 -5Z" fill={p.clothing} />
				<ellipse cx="0" cy="-5" rx="14" ry="5" fill={p.accent} />
				<path
					d="M-9 -6Q-5 -11 -1 -6T7 -6M-8 -3Q-3 -7 2 -3"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<path d="M3 -8L10 -23M7 -6L15 -20" strokeWidth="2" />
				<path d="M-6 3H6" stroke={p.cheek} strokeWidth="3" />
			</>
		)),
		handProp(
			"rolling-pin",
			"Rolling pin",
			(p) => (
				<>
					<path d="M-13 5L8 -18L15 -11L-6 12Z" fill={p.accent} />
					<path
						d="M-13 5L-18 10L-13 15L-7 10M8 -18L13 -23L18 -18L13 -13"
						fill={p.clothing}
					/>
					<path d="M-9 6L9 -13" stroke={p.sclera} strokeWidth="2" />
				</>
			),
			"tools",
		),
		handProp("ice-cream-cone", "Ice cream", (p) => (
			<>
				<path d="M-10 -3L0 15L10 -3Z" fill={p.accent} />
				<path d="M-6 -2L4 7M0 -3L7 3M6 -2L-4 7M0 -3L-7 3" strokeWidth=".9" />
				<path
					d="M-11 -4Q-15 -10 -9 -12Q-11 -21 -2 -20Q7 -24 10 -15Q17 -12 11 -4Z"
					fill={p.cheek}
				/>
				<path
					d="M-5 -14L-2 -16M3 -10L6 -11"
					stroke={p.sclera}
					strokeWidth="2"
				/>
			</>
		)),
		handProp(
			"kitchen-whisk",
			"Whisk",
			(p) => (
				<>
					<path
						d="M-2 -2C-19 -14 -9 -24 0 -21C9 -24 19 -14 2 -2Z"
						fill={p.sclera}
					/>
					<path d="M0 -21Q-10 -13 0 -2Q10 -13 0 -21V-2" />
					<rect x="-3" y="-2" width="6" height="17" rx="3" fill={p.clothing} />
					<circle cx="0" cy="11" r="1" fill={p.accent} stroke="none" />
				</>
			),
			"tools",
		),
		handProp("pizza-slice", "Pizza slice", (p) => (
			<>
				<path d="M-12 -12Q0 -18 12 -12L0 15Z" fill={p.accent} />
				<path d="M-13 -12Q0 -20 13 -12" stroke={p.clothing} strokeWidth="5" />
				<circle cx="-4" cy="-6" r="3" fill={p.cheek} />
				<circle cx="5" cy="-5" r="2.5" fill={p.cheek} />
				<circle cx="0" cy="4" r="2.5" fill={p.cheek} />
			</>
		)),
	],
	cosmos: [
		handProp("asteroid-sample", "Asteroid sample", (p) => (
			<>
				<path
					d="M-12 -11L-4 -21L8 -18L15 -7L11 6L1 12L-12 6L-16 -4Z"
					fill={p.clothing}
				/>
				<circle cx="-4" cy="-9" r="4" fill={p.accent} />
				<circle cx="7" cy="0" r="3" fill={p.accent} />
				<path
					d="M3 -16L7 -13M-10 1L-6 4M11 -8L12 -5"
					stroke={p.sclera}
					strokeWidth="2"
				/>
			</>
		)),
		handProp("orbit-globe", "Orbit globe", (p) => (
			<>
				<circle cx="0" cy="-9" r="10" fill={p.accent} />
				<ellipse cx="0" cy="-9" rx="5" ry="10" />
				<path
					d="M-10 -9H10M-7 -16Q0 -12 7 -16M-7 -2Q0 -6 7 -2"
					stroke={p.sclera}
				/>
				<path
					d="M-9 -20Q-19 -7 -7 3Q4 11 12 -1M0 6V11M-8 12H8"
					stroke={p.clothing}
					strokeWidth="2.5"
				/>
			</>
		)),
		handProp(
			"lunar-rover",
			"Lunar rover",
			(p) => (
				<>
					<path d="M-12 -5H11L14 3H-15Z" fill={p.clothing} />
					<path
						d="M-7 -5V-13H5V-5M7 -9L12 -18M8 -22Q12 -14 16 -21Z"
						fill={p.sclera}
					/>
					<path d="M-8 3V7M8 3V7M-7 -10H5" stroke={p.accent} strokeWidth="2" />
					<circle cx="-10" cy="7" r="4" fill={p.accent} />
					<circle cx="10" cy="7" r="4" fill={p.accent} />
					<circle cx="0" cy="7" r="3" fill={p.accent} />
				</>
			),
			"device",
		),
		handProp(
			"stargazing-telescope",
			"Telescope",
			(p) => (
				<>
					<path d="M-7 -13L-2 -2L15 -10L10 -21Z" fill={p.clothing} />
					<path
						d="M-8 -10L-4 -3L-10 0L-14 -7ZM10 -21L15 -10L18 -12L13 -23Z"
						fill={p.accent}
					/>
					<path d="M3 -5V3M3 1L-7 14M3 1L12 14M3 1V14" strokeWidth="2" />
				</>
			),
			"search",
		),
		handProp(
			"satellite-model",
			"Satellite model",
			(p) => (
				<>
					<path d="M-5 -8H5V7H-5Z" fill={p.accent} />
					<path d="M-16 -6H-6V5H-16ZM6 -6H16V5H6Z" fill={p.clothing} />
					<path
						d="M-11 -6V5M11 -6V5M-16 -1H-6M6 -1H16"
						stroke={p.sclera}
						strokeWidth=".8"
					/>
					<path d="M0 -8V-14M-6 -19Q0 -9 6 -19ZM0 7V12" fill={p.sclera} />
					<circle cx="0" cy="-19" r="1.5" fill={p.accent} />
				</>
			),
			"device",
		),
	],
	pro: [
		handProp(
			"leather-folio",
			"Document folio",
			(p) => (
				<>
					<path d="M-12 -19H10V11H-12Z" fill={p.sclera} />
					<path d="M-14 -16H13V13H-14Z" fill={p.clothing} />
					<path d="M-14 -10H13M-10 -16V13" stroke={p.accent} />
					<path d="M6 -3H15V4H6Z" fill={p.accent} />
					<circle cx="11" cy=".5" r="1.2" fill={p.sclera} />
				</>
			),
			"files",
		),
		handProp("wax-seal-stamp", "Wax seal stamp", (p) => (
			<>
				<path d="M-6 -15Q-7 -23 0 -23Q7 -23 6 -15L3 -5H-3Z" fill={p.clothing} />
				<path d="M-10 -5H10V1H-10Z" fill={p.accent} />
				<path
					d="M-12 6L-7 2L-1 4L5 2L11 6L8 11L2 12L-4 10L-10 12Z"
					fill={p.cheek}
				/>
				<path d="M-4 5H4L0 10Z" fill={p.sclera} />
			</>
		)),
		handProp("folded-newspaper", "Newspaper", (p) => (
			<>
				<path
					d="M-13 -18H10V10H-9Q-13 10 -13 6ZM10 -12H14V7Q14 10 10 10"
					fill={p.sclera}
				/>
				<path d="M-9 -13H6M-9 -9H6" stroke={p.clothing} strokeWidth="2" />
				<path d="M-9 -4H-2V3H-9Z" fill={p.accent} />
				<path d="M2 -4H6M2 0H6M-9 6H6M10 -12V10" stroke={p.clothing} />
			</>
		)),
		handProp("fountain-pen", "Fountain pen", (p) => (
			<>
				<path d="M-3 9L6 -15L13 -12L3 12Z" fill={p.clothing} />
				<path
					d="M-3 9L-5 17L3 12ZM6 -15L8 -20L15 -17L13 -12Z"
					fill={p.accent}
				/>
				<path d="M9 -15L5 -5M-3 13L0 11" stroke={p.sclera} />
				<path d="M-13 13Q-13 7 -9 9T-5 5" />
			</>
		)),
		handProp("pocket-watch", "Pocket watch", (p) => (
			<>
				<path d="M0 -14V-18M-3 -19H3M10 -7Q20 -9 15 -18Q11 -24 7 -20" />
				<circle cx="0" cy="0" r="13" fill={p.accent} />
				<circle cx="0" cy="0" r="10" fill={p.sclera} />
				<path d="M0 -7V0L5 3M-7 0H-6M7 0H6M0 7V6" />
				<circle cx="0" cy="0" r="1.5" fill={p.clothing} />
			</>
		)),
	],
	office: [
		handProp("desk-stapler", "Stapler", (p) => (
			<>
				<path d="M-14 2H13V9H-14Z" fill={p.sclera} />
				<path
					d="M-12 2V-8Q-12 -12 -7 -11L14 -5L12 1L-7 -3V2Z"
					fill={p.clothing}
				/>
				<path d="M-6 -8L9 -4" stroke={p.accent} strokeWidth="3" />
				<circle cx="-9" cy="0" r="2" fill={p.accent} />
			</>
		)),
		handProp("binder-clip-stack", "Binder clips", (p) => (
			<>
				<path d="M-14 -3H1L4 11H-17ZM-2 -13H12L15 1H-5Z" fill={p.clothing} />
				<path
					d="M-10 -3V-13Q-6 -18 -3 -13V-3M2 -13V-22Q6 -26 10 -22V-13"
					stroke={p.accent}
					strokeWidth="2"
				/>
				<path d="M-14 7H1M-2 -3H12" stroke={p.sclera} />
			</>
		)),
		handProp("correction-tape", "Correction tape", (p) => (
			<>
				<path
					d="M-12 7Q-19 -5 -8 -14Q4 -25 13 -13Q19 -4 7 4L-8 11Z"
					fill={p.accent}
				/>
				<circle cx="5" cy="-9" r="6" fill={p.sclera} />
				<circle cx="-6" cy="1" r="4" fill={p.clothing} />
				<path d="M-8 8L-14 11L-10 15L-4 11Z" fill={p.sclera} />
				<circle cx="5" cy="-9" r="2" fill={p.clothing} />
			</>
		)),
		handProp(
			"document-folder",
			"Document folder",
			(p) => (
				<>
					<path d="M-14 -12H-3L0 -8H14V12H-14Z" fill={p.clothing} />
					<path d="M-10 -17H8L12 -13V6H-10Z" fill={p.sclera} />
					<path d="M8 -17V-12H12M-6 -10H5M-6 -6H7" />
					<path d="M-15 -3H15L12 12H-12Z" fill={p.accent} />
				</>
			),
			"files",
		),
		handProp(
			"sticky-note-pad",
			"Sticky notes",
			(p) => (
				<>
					<path d="M-13 -10H12V13H-13Z" fill={p.clothing} />
					<path d="M-11 -16H14V3L6 10H-11Z" fill={p.accent} />
					<path d="M6 10V3H14" fill={p.sclera} />
					<path d="M-6 -8L-3 -5L3 -11M-6 0H6M-13 10H-11" />
				</>
			),
			"tasks",
		),
	],
	workshop: [
		handProp(
			"craft-scissors",
			"Craft scissors",
			(p) => (
				<>
					<path
						d="M0 -2L-10 -22Q-15 -18 -3 2M0 -2L10 -22Q15 -18 3 2"
						fill={p.sclera}
					/>
					<path d="M-2 0L-6 6M2 0L6 6" stroke={p.clothing} strokeWidth="3" />
					<ellipse cx="-7" cy="7" rx="5" ry="6" fill={p.accent} />
					<ellipse cx="7" cy="7" rx="5" ry="6" fill={p.accent} />
					<ellipse cx="-7" cy="7" rx="2" ry="3" fill={p.background} />
					<ellipse cx="7" cy="7" rx="2" ry="3" fill={p.background} />
					<circle cx="0" cy="-1" r="2" fill={p.clothing} />
				</>
			),
			"tools",
		),
		handProp(
			"crochet-hook",
			"Crochet hook",
			(p) => (
				<>
					<path
						d="M0 11V-16Q0 -23 5 -21Q8 -20 4 -17"
						stroke={p.accent}
						strokeWidth="3"
					/>
					<path d="M-3 0H3V12H-3Z" fill={p.clothing} />
					<path
						d="M-11 -5Q-8 -15 -3 -8Q1 -15 5 -8Q12 -14 13 -5L10 3H-9Z"
						fill={p.cheek}
					/>
					<path d="M-9 -5L-6 -1L-3 -5L0 -1L3 -5L6 -1L9 -5" stroke={p.sclera} />
				</>
			),
			"tools",
		),
		handProp(
			"carving-chisel",
			"Carving chisel",
			(p) => (
				<>
					<path d="M-5 -22H5L3 -5H-3Z" fill={p.sclera} />
					<path d="M-5 -22L-4 -18H4L5 -22" fill={p.accent} />
					<path d="M-3 -5H3V-1H-3Z" fill={p.accent} />
					<rect x="-5" y="-1" width="10" height="14" rx="4" fill={p.clothing} />
					<path d="M-1 3V9M8 11Q16 8 10 3" stroke={p.accent} strokeWidth="2" />
				</>
			),
			"tools",
		),
		handProp(
			"claw-hammer",
			"Claw hammer",
			(p) => (
				<>
					<path d="M-3 -11H3V13H-3Z" fill={p.clothing} />
					<path d="M-12 -19H5L13 -15L15 -8L9 -12H3V-8H-12Z" fill={p.accent} />
					<path d="M-14 -20H-10V-7H-14Z" fill={p.sclera} />
					<path d="M-3 5H3M-3 9H3" stroke={p.accent} />
				</>
			),
			"tools",
		),
		handProp(
			"artist-palette",
			"Paint palette",
			(p) => (
				<>
					<path
						d="M-14 0C-19 -18 11 -22 15 -8Q17 -1 8 0Q2 2 7 7Q12 16 -1 14Q-12 13 -14 0Z"
						fill={p.accent}
					/>
					<circle cx="-3" cy="5" r="3" fill={p.background} />
					<circle cx="-8" cy="-7" r="3" fill={p.cheek} stroke="none" />
					<circle cx="0" cy="-12" r="3" fill={p.clothing} stroke="none" />
					<circle cx="9" cy="-9" r="3" fill={p.sclera} stroke="none" />
					<path d="M8 13L14 -6" stroke={p.clothing} strokeWidth="3" />
				</>
			),
			"image",
		),
		handProp(
			"tape-measure",
			"Tape measure",
			(p) => (
				<>
					<rect
						x="-14"
						y="-12"
						width="24"
						height="25"
						rx="6"
						fill={p.clothing}
					/>
					<circle cx="-2" cy="0" r="7" fill={p.accent} />
					<path d="M10 1H18V8H10Z" fill={p.sclera} />
					<path d="M18 -1V9M13 1V4M16 1V4M-4 -4H0V4H-4" />
					<path d="M-7 -12V-16H3V-12" />
				</>
			),
			"tools",
		),
	],
	ocean: [
		handProp("scuba-fin", "Scuba fin", (p) => (
			<>
				<path d="M-6 -21H6L15 9Q0 15 -15 9Z" fill={p.clothing} />
				<path d="M-5 -20H5L7 -8Q0 -4 -7 -8Z" fill={p.accent} />
				<path d="M-4 -16H4L5 -10Q0 -8 -5 -10Z" fill={p.ink} />
				<path
					d="M-5 -3L-9 8M0 -2V10M5 -3L9 8"
					stroke={p.sclera}
					strokeWidth="2"
				/>
			</>
		)),
		handProp("message-in-bottle", "Message in a bottle", (p) => (
			<>
				<path d="M-4 -17H4V-8L9 -2V10Q0 14 -9 10V-2L-4 -8Z" fill={p.sclera} />
				<path d="M-5 -22H5V-16H-5Z" fill={p.clothing} />
				<path d="M-5 -1L4 -3L6 7L-3 9Z" fill={p.accent} />
				<path d="M-2 1L3 0M-1 4L4 3" stroke={p.clothing} />
			</>
		)),
		handProp("rescue-lifering", "Lifering", (p) => (
			<>
				<circle cx="0" cy="-4" r="14" fill={p.sclera} />
				<path
					d="M-5 -17H5V-10H-5ZM-5 2H5V9H-5ZM-13 -9H-6V1H-13ZM6 -9H13V1H6Z"
					fill={p.cheek}
				/>
				<circle cx="0" cy="-4" r="6" fill={p.background} />
				<path
					d="M-13 -13Q-22 0 -10 12M13 -13Q22 0 10 12"
					stroke={p.accent}
					strokeWidth="2"
				/>
			</>
		)),
		handProp("scallop-shell", "Seashell", (p) => (
			<>
				<path
					d="M-5 11L-15 -3Q-19 -10 -12 -11Q-13 -19 -5 -17Q0 -24 5 -17Q13 -19 12 -11Q19 -10 15 -3L5 11Z"
					fill={p.cheek}
				/>
				<path
					d="M0 9V-17M-3 9L-10 -12M3 9L10 -12M-5 8L-14 -4M5 8L14 -4M-5 11H5V14H-5Z"
					fill={p.accent}
				/>
			</>
		)),
		handProp("dip-net", "Dip net", (p) => (
			<>
				<path d="M1 -5L-4 14" stroke={p.clothing} strokeWidth="4" />
				<path d="M-9 -13L-4 2H8L13 -13Z" fill={p.sclera} />
				<path
					d="M-6 -11L6 1M0 -11L10 -2M8 -11L-4 1M2 -11L-7 -2"
					stroke={p.clothing}
					strokeWidth=".8"
				/>
				<ellipse cx="2" cy="-13" rx="12" ry="6" fill="none" strokeWidth="2.5" />
			</>
		)),
	],
	fantasy: [
		handProp("dragon-egg", "Dragon egg", (p) => (
			<>
				<path
					d="M0 -22Q16 -9 13 4Q9 14 0 13Q-9 14 -13 4Q-16 -9 0 -22Z"
					fill={p.accent}
				/>
				<path
					d="M-5 -13L0 -9L5 -13M-9 -5L-4 0L0 -5L4 0L9 -5M-10 4L-5 8L0 3L5 8L10 4"
					fill={p.clothing}
				/>
				<path d="M-3 -17L0 -20" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		handProp("fairy-lantern", "Fairy lantern", (p) => (
			<>
				<path d="M0 -17V-22Q7 -25 8 -18" strokeWidth="2" />
				<path d="M-10 -7Q-3 -17 0 -17Q3 -17 10 -7L7 8H-7Z" fill={p.sclera} />
				<path
					d="M0 -6L2 -2L6 -1L3 2L4 6L0 4L-4 6L-3 2L-6 -1L-2 -2Z"
					fill={p.accent}
				/>
				<path
					d="M-10 -7H10M-7 8H7M-7 -7L-4 8M7 -7L4 8"
					stroke={p.clothing}
					strokeWidth="2"
				/>
				<path d="M0 8V13M-3 11L0 14L3 11" stroke={p.cheek} />
			</>
		)),
		handProp("royal-scepter", "Royal scepter", (p) => (
			<>
				<path d="M0 -7V12" stroke={p.clothing} strokeWidth="4" />
				<path
					d="M-9 -15L-10 -22L-4 -17L0 -24L4 -17L10 -22L9 -15Z"
					fill={p.accent}
				/>
				<path d="M-9 -15H9L5 -8H-5Z" fill={p.accent} />
				<circle cx="0" cy="-13" r="3" fill={p.cheek} />
				<path d="M-2 8H2M-3 13H3" stroke={p.accent} strokeWidth="3" />
			</>
		)),
		handProp(
			"knight-shield",
			"Knight shield",
			(p) => (
				<>
					<path
						d="M0 -19L14 -13V0Q12 9 0 15Q-12 9 -14 0V-13Z"
						fill={p.clothing}
					/>
					<path
						d="M0 -14L10 -10V-1Q9 6 0 10Q-9 6 -10 -1V-10Z"
						fill={p.accent}
					/>
					<path d="M0 -10V6M-6 -4H6" stroke={p.sclera} strokeWidth="3" />
				</>
			),
			"shield",
		),
		handProp("crystal-sword", "Crystal sword", (p) => (
			<>
				<path d="M-4 2L-5 -13L0 -23L5 -13L4 2Z" fill={p.sclera} />
				<path d="M0 -23V2L5 -13Z" fill={p.accent} />
				<path d="M-10 1L-9 6L-3 4V13H3V4L9 6L10 1Z" fill={p.clothing} />
				<circle cx="0" cy="14" r="3" fill={p.accent} />
			</>
		)),
	],
	garden: [
		handProp("seed-packet", "Seed packet", (p) => (
			<>
				<path d="M-12 -20H12V11H-12Z" fill={p.sclera} />
				<path d="M-12 -20H12V-13H-12Z" fill={p.clothing} />
				<path
					d="M0 7V-5M0 0Q-10 1 -8 -7Q-1 -8 0 0ZM0 -3Q2 -13 9 -9Q10 -2 0 -3Z"
					fill={p.accent}
				/>
				<path d="M-7 8H7" stroke={p.cheek} strokeWidth="2" />
			</>
		)),
		handProp(
			"pruning-shears",
			"Pruning shears",
			(p) => (
				<>
					<path
						d="M0 -1Q-15 -12 -7 -22Q-8 -9 3 -5M0 -1Q15 -12 7 -22Q9 -6 3 3Z"
						fill={p.sclera}
					/>
					<path
						d="M-2 0L-10 12M3 1L10 12"
						stroke={p.clothing}
						strokeWidth="6"
					/>
					<path d="M-2 3Q1 8 5 4" stroke={p.accent} strokeWidth="2" />
					<circle cx="0" cy="-1" r="3" fill={p.accent} />
				</>
			),
			"tools",
		),
		handProp("seedling-pot", "Seedling pot", (p) => (
			<>
				<path d="M-10 -3H10L7 12H-7Z" fill={p.clothing} />
				<path d="M-12 -6H12V-1H-12Z" fill={p.accent} />
				<path
					d="M0 -6V-17M0 -11Q-13 -8 -11 -20Q0 -22 0 -11ZM0 -16Q3 -26 11 -21Q12 -13 0 -16Z"
					fill={p.accent}
				/>
				<path d="M-5 3H5" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		handProp("watering-can", "Watering can", (p) => (
			<>
				<path d="M4 -8Q18 -22 17 -6Q16 0 8 2" strokeWidth="3" />
				<path d="M-10 -8H8V9Q0 14 -10 9Z" fill={p.clothing} />
				<path d="M-10 3L-17 -10L-12 -13L-5 -1" fill={p.accent} />
				<path d="M-19 -12L-11 -16L-9 -12L-17 -8Z" fill={p.sclera} />
				<ellipse cx="-1" cy="-8" rx="9" ry="3" fill={p.accent} />
			</>
		)),
		handProp(
			"garden-trowel",
			"Garden trowel",
			(p) => (
				<>
					<path d="M-3 -2Q-14 -14 0 -23Q14 -14 3 -2Z" fill={p.sclera} />
					<path d="M0 -18V1" stroke={p.clothing} />
					<rect x="-4" y="0" width="8" height="15" rx="3" fill={p.accent} />
					<circle cx="0" cy="10" r="1.5" fill={p.clothing} />
				</>
			),
			"tools",
		),
	],
	music: [
		handProp(
			"saxophone",
			"Saxophone",
			(p) => (
				<>
					<path
						d="M0 -19H7V-15H3V3Q3 9 -3 9Q-9 9 -9 3V-5H-14V3Q-14 15 -3 15Q9 15 9 3V-13Q9 -20 5 -23H0Z"
						fill={p.accent}
					/>
					<path d="M-17 -9H-6V-4H-17Z" fill={p.clothing} />
					<path d="M0 -23H-5V-19H0" fill={p.clothing} />
					<path d="M3 -10H7M3 -5H7M3 0H7" stroke={p.sclera} strokeWidth="2" />
				</>
			),
			"music",
		),
		handProp(
			"snare-drum",
			"Snare drum",
			(p) => (
				<>
					<path d="M-14 -6H14V9Q0 16 -14 9Z" fill={p.clothing} />
					<ellipse cx="0" cy="-6" rx="14" ry="5" fill={p.sclera} />
					<path
						d="M-12 -3L-7 10L0 -1L7 10L12 -3"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path
						d="M-12 -22L8 -7M12 -22L-8 -7"
						stroke={p.accent}
						strokeWidth="2.5"
					/>
					<path d="M-13 9Q0 15 13 9" stroke={p.sclera} strokeWidth="2" />
				</>
			),
			"music",
		),
		handProp(
			"violin",
			"Violin",
			(p) => (
				<>
					<path
						d="M-3 -10Q-10 -13 -11 -6Q-11 -2 -6 0Q-15 5 -11 11Q-5 18 3 11Q7 5 0 0Q6 -2 5 -7Q5 -12 0 -10Z"
						fill={p.accent}
					/>
					<path d="M-3 5L-1 -21H3L1 5Z" fill={p.clothing} />
					<path
						d="M-3 -21H5M-7 7H1M11 -20L8 14"
						stroke={p.clothing}
						strokeWidth="2"
					/>
					<path d="M-8 1Q-10 4 -7 5M2 1Q4 4 1 5M0 -17L-2 9" stroke={p.sclera} />
				</>
			),
			"music",
		),
		handProp(
			"mini-ukulele",
			"Ukulele",
			(p) => (
				<>
					<path
						d="M-3 -9Q-12 -13 -12 -5Q-12 -1 -9 1Q-17 12 -5 15Q7 18 9 6Q10 -1 3 -3L5 -9Z"
						fill={p.accent}
					/>
					<path d="M-2 2L3 -19L8 -18L3 3Z" fill={p.clothing} />
					<path d="M2 -22L10 -20L8 -14L1 -16Z" fill={p.accent} />
					<circle cx="-3" cy="5" r="4" fill={p.ink} />
					<path d="M-7 11L1 13M-3 10L5 -17" stroke={p.sclera} />
				</>
			),
			"music",
		),
		handProp(
			"stage-microphone",
			"Microphone",
			(p) => (
				<>
					<path d="M-4 -4H4L3 12H-3Z" fill={p.clothing} />
					<circle cx="0" cy="-12" r="9" fill={p.sclera} />
					<path
						d="M-7 -16H7M-8 -12H8M-6 -8H6M-3 -20V-4M3 -20V-4"
						stroke={p.clothing}
						strokeWidth=".8"
					/>
					<path d="M0 12Q7 17 13 11" />
					<path d="M-4 1H4" stroke={p.accent} strokeWidth="3" />
				</>
			),
			"music",
		),
		handProp(
			"tambourine",
			"Tambourine",
			(p) => (
				<>
					<circle cx="0" cy="-2" r="14" fill={p.clothing} />
					<circle cx="0" cy="-2" r="9" fill={p.sclera} />
					<ellipse cx="0" cy="-15" rx="4" ry="2.5" fill={p.accent} />
					<ellipse cx="0" cy="11" rx="4" ry="2.5" fill={p.accent} />
					<ellipse cx="-13" cy="-2" rx="2.5" ry="4" fill={p.accent} />
					<ellipse cx="13" cy="-2" rx="2.5" ry="4" fill={p.accent} />
					<path d="M-5 -5Q0 -9 5 -5" stroke={p.accent} />
				</>
			),
			"music",
		),
	],
	adventure: [
		handProp("basketball", "Basketball", (p) => (
			<>
				<circle cx="0" cy="-4" r="14" fill={p.accent} />
				<path
					d="M-14 -4H14M0 -18V10M-10 -14Q4 -4 -10 6M10 -14Q-4 -4 10 6"
					stroke={p.clothing}
					strokeWidth="1.8"
				/>
			</>
		)),
		handProp("skateboard", "Skateboard", (p) => (
			<>
				<rect x="-7" y="-23" width="14" height="36" rx="7" fill={p.clothing} />
				<path d="M-7 -13H7M-7 4H7" stroke={p.sclera} strokeWidth="2" />
				<path d="M-3 -10L3 -10L-1 -3H4L-4 4L-1 -2H-5Z" fill={p.accent} />
				<path
					d="M-10 -15V-11M10 -15V-11M-10 2V6M10 2V6"
					stroke={p.accent}
					strokeWidth="4"
				/>
			</>
		)),
		handProp("boxing-glove", "Boxing glove", (p) => (
			<>
				<path
					d="M-9 3Q-16 -1 -13 -8Q-12 -12 -7 -9V-15Q-7 -23 3 -22Q14 -21 13 -10V-1L8 5Z"
					fill={p.cheek}
				/>
				<path d="M-9 3H9V12H-9Z" fill={p.clothing} />
				<path d="M-7 -9L-5 -3M-6 7H6" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		handProp("badminton-shuttle", "Shuttlecock", (p) => (
			<>
				<path
					d="M-4 3L-14 -17L-8 -20L-4 -18L0 -22L4 -18L8 -20L14 -17L4 3Z"
					fill={p.sclera}
				/>
				<path
					d="M-8 -18L-2 3M0 -21V3M8 -18L2 3M-10 -9H10M-7 -3H7"
					stroke={p.clothing}
				/>
				<path d="M-5 3H5V8Q0 15 -5 8Z" fill={p.accent} />
			</>
		)),
		handProp("hiking-boot", "Hiking boot", (p) => (
			<>
				<path d="M-10 -19H2V-7L12 -3Q17 -1 15 7H-13V-6Z" fill={p.clothing} />
				<path d="M-13 6H16V11H-14Z" fill={p.accent} />
				<path
					d="M-8 -19H2V-13H-8M-3 -11L2 -8M-5 -7L0 -4M-7 -3L-2 0"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<path d="M-10 9V12M-3 9V12M4 9V12M11 9V12" />
			</>
		)),
		handProp("climbing-carabiner", "Climbing carabiner", (p) => (
			<>
				<path
					d="M-9 4L-4 -14Q0 -24 8 -19Q14 -15 10 -8L6 5Q0 17 -7 10Q-10 8 -9 4Z"
					stroke={p.ink}
					strokeWidth="7"
				/>
				<path
					d="M-9 4L-4 -14Q0 -24 8 -19Q14 -15 10 -8L6 5Q0 17 -7 10Q-10 8 -9 4Z"
					stroke={p.accent}
					strokeWidth="4"
				/>
				<path d="M-9 4L-4 -13" stroke={p.sclera} strokeWidth="4" />
				<path
					d="M-10 -2L-5 0M-9 -5L-4 -3"
					stroke={p.clothing}
					strokeWidth="2"
				/>
			</>
		)),
		handProp("tennis-racket", "Tennis racket", (p) => (
			<>
				<ellipse cx="1" cy="-12" rx="10" ry="12" fill={p.sclera} />
				<path
					d="M-5 -20V-4M1 -23V-1M7 -20V-4M-8 -17H10M-9 -11H11M-7 -5H9"
					stroke={p.clothing}
					strokeWidth=".8"
				/>
				<ellipse
					cx="1"
					cy="-12"
					rx="10"
					ry="12"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path
					d="M-3 -1L-1 5H3L5 -1M1 5V15"
					stroke={p.clothing}
					strokeWidth="3"
				/>
				<path d="M-1 9H3M-1 12H3" stroke={p.accent} />
			</>
		)),
		handProp("trail-compass", "Compass", (p) => (
			<>
				<circle cx="0" cy="-18" r="3" fill={p.clothing} />
				<circle cx="0" cy="-1" r="14" fill={p.accent} />
				<circle cx="0" cy="-1" r="10" fill={p.sclera} />
				<path d="M-6 5L-2 -3L6 -7L2 1Z" fill={p.clothing} />
				<path d="M-2 -3L6 -7L2 1Z" fill={p.cheek} />
				<path d="M0 -10V-8M0 8V6M-9 -1H-7M9 -1H7" />
			</>
		)),
		handProp("folded-trail-map", "Trail map", (p) => (
			<>
				<path
					d="M-16 -15L-5 -18L5 -13L16 -16V9L5 12L-5 7L-16 10Z"
					fill={p.sclera}
				/>
				<path d="M-5 -18V7M5 -13V12" stroke={p.clothing} />
				<path
					d="M-13 4Q-8 -10 -2 -4T12 -9"
					stroke={p.cheek}
					strokeWidth="2"
					strokeDasharray="2 3"
				/>
				<path d="M-13 -7L-9 -13L-5 -7M7 5L10 0L13 5" fill={p.accent} />
			</>
		)),
		handProp("camp-lantern", "Camping lantern", (p) => (
			<>
				<path d="M-7 -14V-18Q0 -26 7 -18V-14" strokeWidth="2" />
				<path d="M-10 -12H10L12 9H-12Z" fill={p.sclera} />
				<path d="M0 -9Q-8 1 0 6Q8 1 0 -9Z" fill={p.accent} />
				<path
					d="M-10 -13H10M-11 10H11M-7 -11V9M7 -11V9"
					stroke={p.clothing}
					strokeWidth="3"
				/>
				<rect x="-12" y="10" width="24" height="5" rx="2" fill={p.clothing} />
			</>
		)),
	],
	arcade: [
		handProp(
			"game-controller",
			"Game controller",
			(p) => (
				<>
					<path
						d="M-7 -11H7Q14 -13 16 -3L18 7Q18 14 12 9L6 4H-6L-12 9Q-18 14 -18 7L-16 -3Q-14 -13 -7 -11Z"
						fill={p.clothing}
					/>
					<path d="M-9 -7V1M-13 -3H-5" stroke={p.sclera} strokeWidth="3" />
					<circle cx="9" cy="-6" r="2" fill={p.accent} />
					<circle cx="13" cy="-2" r="2" fill={p.cheek} />
					<path d="M-1 -6H2" stroke={p.sclera} />
				</>
			),
			"device",
		),
		handProp("puzzle-cube", "Puzzle cube", (p) => (
			<>
				<path d="M0 -20L14 -12V5L0 13L-14 5V-12Z" fill={p.accent} />
				<path d="M-14 -12L0 -4L14 -12V5L0 13V-4" fill={p.clothing} />
				<path
					d="M-14 -4L0 4L14 -4M-7 -16L7 -8M7 -16L-7 -8M-7 -8V9M7 -8V9"
					stroke={p.sclera}
				/>
				<path d="M1 -3L6 -6V1L1 4Z" fill={p.cheek} stroke="none" />
			</>
		)),
		handProp("chess-knight", "Chess knight", (p) => (
			<>
				<path
					d="M-8 5Q-4 -4 2 -5L-8 -5L-12 -10L-4 -18L-1 -24L4 -20Q14 -16 9 5Z"
					fill={p.sclera}
				/>
				<path d="M-11 5H12V10H-11ZM-13 10H14V13H-13Z" fill={p.clothing} />
				<path d="M5 -17Q9 -9 4 2" stroke={p.accent} strokeWidth="3" />
				<circle cx="-1" cy="-14" r="1.5" fill={p.ink} stroke="none" />
			</>
		)),
		handProp("arcade-joystick", "Joystick", (p) => (
			<>
				<path d="M-15 0H13L17 12H-17Z" fill={p.clothing} />
				<path d="M-6 -11V4" strokeWidth="3" />
				<circle cx="-6" cy="-15" r="6" fill={p.cheek} />
				<ellipse cx="7" cy="5" rx="4" ry="2" fill={p.accent} />
				<path d="M-13 9H13" stroke={p.sclera} />
			</>
		)),
		handProp("playing-cards", "Playing cards", (p) => (
			<>
				<path d="M-16 -11L2 -19L14 7L-4 15Z" fill={p.clothing} />
				<path d="M-11 -13L8 -17L14 11L-5 15Z" fill={p.accent} />
				<rect x="-6" y="-17" width="20" height="30" rx="2" fill={p.sclera} />
				<path
					d="M4 -7Q-3 -13 -3 -5Q-2 -1 4 3Q10 -1 11 -5Q11 -13 4 -7Z"
					fill={p.cheek}
				/>
				<path d="M-2 -13H0M8 9H10" stroke={p.cheek} />
			</>
		)),
	],
	pirates: [
		handProp("treasure-chest-hand", "Treasure chest", (p) => (
			<>
				<path
					d="M-14 -4Q-14 -17 0 -17Q14 -17 14 -4V11H-14Z"
					fill={p.clothing}
				/>
				<path
					d="M-14 -3H14M-8 -15V11M8 -15V11"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path d="M-3 -4H3V3H-3Z" fill={p.accent} />
				<path d="M-11 7H-9M9 7H11" stroke={p.sclera} />
			</>
		)),
		handProp("grappling-hook", "Grappling hook", (p) => (
			<>
				<path
					d="M0 -12V5Q-10 7 -12 -3M0 5Q10 7 12 -3M0 5V10"
					stroke={p.sclera}
					strokeWidth="4"
				/>
				<path d="M-15 -1L-12 -7L-8 -2M8 -2L12 -7L15 -1" fill={p.accent} />
				<circle cx="0" cy="-16" r="4" fill={p.accent} />
				<path
					d="M0 -20Q-13 -25 -13 -16Q-13 -9 -8 -10"
					stroke={p.clothing}
					strokeWidth="2.5"
				/>
			</>
		)),
		handProp("treasure-map-scroll", "Treasure scroll", (p) => (
			<>
				<path d="M-11 -17H10V10H-11Z" fill={p.sclera} />
				<path d="M-14 -20H13V-14H-14ZM-14 7H13V13H-14Z" fill={p.accent} />
				<path
					d="M-7 4Q6 5 2 -1T-6 -7"
					stroke={p.clothing}
					strokeDasharray="2 2"
				/>
				<path d="M2 -11L7 -6M7 -11L2 -6" stroke={p.cheek} strokeWidth="2" />
				<path d="M-15 -18H-12M11 10H14" />
			</>
		)),
		handProp("treasure-key", "Treasure key", (p) => (
			<>
				<path d="M-2 -4H3V14H-2ZM3 6H10V10H6V14H3" fill={p.accent} />
				<path
					d="M-8 -14Q-8 -21 0 -18Q8 -21 8 -14Q13 -7 5 -5Q0 0 -5 -5Q-13 -7 -8 -14Z"
					fill={p.accent}
				/>
				<circle cx="0" cy="-11" r="3" fill={p.clothing} />
			</>
		)),
		handProp(
			"brass-spyglass",
			"Brass spyglass",
			(p) => (
				<>
					<path d="M-15 6L-11 13L-3 8L-7 1Z" fill={p.clothing} />
					<path d="M-9 0L-3 10L8 2L2 -8Z" fill={p.accent} />
					<path d="M0 -10L8 3L17 -3L9 -16Z" fill={p.accent} />
					<path d="M9 -16L17 -3L20 -5L12 -18Z" fill={p.sclera} />
					<path d="M4 -8L11 -13M-5 1L0 -3" stroke={p.sclera} />
				</>
			),
			"search",
		),
	],
	arcane: [
		handProp("spellbook-hand", "Spellbook", (p) => (
			<>
				<path
					d="M-15 -17Q-7 -21 0 -16Q7 -21 15 -17V9Q7 6 0 12Q-7 6 -15 9Z"
					fill={p.clothing}
				/>
				<path
					d="M-12 -14Q-6 -17 0 -12Q6 -17 12 -14V6Q6 3 0 9Q-6 3 -12 6Z"
					fill={p.sclera}
				/>
				<path
					d="M0 -12V9M-9 -8H-4M-9 -4H-4M5 -8L8 -5L5 -2L2 -5Z"
					stroke={p.accent}
					strokeWidth="2"
				/>
			</>
		)),
		handProp("hourglass-hand", "Hourglass", (p) => (
			<>
				<path
					d="M-9 -17H9Q10 -7 2 -4Q10 0 9 10H-9Q-10 0 -2 -4Q-10 -7 -9 -17Z"
					fill={p.sclera}
				/>
				<path d="M-7 -13H7L0 -6ZM-7 8L0 1L7 8Z" fill={p.accent} />
				<path d="M-12 -21H12V-17H-12ZM-12 10H12V14H-12Z" fill={p.clothing} />
				<path d="M0 -5V0" stroke={p.accent} strokeWidth="2" />
			</>
		)),
		handProp("crystal-orb", "Crystal orb", (p) => (
			<>
				<circle cx="0" cy="-8" r="12" fill={p.cheek} />
				<path d="M-7 -13Q-3 -19 3 -16" stroke={p.sclera} strokeWidth="3" />
				<path d="M-8 2H8L11 11H-11Z" fill={p.clothing} />
				<path
					d="M-13 11H13M0 -8L2 -5L5 -4L2 -2L0 1L-2 -2L-5 -4L-2 -5Z"
					fill={p.accent}
				/>
			</>
		)),
		handProp("star-wand", "Star wand", (p) => (
			<>
				<path d="M-8 14L5 -12" stroke={p.clothing} strokeWidth="3" />
				<path
					d="M5 -23L8 -17L15 -16L10 -11L11 -4L5 -7L-1 -4L0 -11L-5 -16L2 -17Z"
					fill={p.accent}
				/>
				<path d="M-13 -11V-5M-16 -8H-10M12 4V8M10 6H14" stroke={p.sclera} />
			</>
		)),
		handProp("potion-flask", "Potion flask", (p) => (
			<>
				<path
					d="M-5 -14V-8Q-15 -4 -13 6Q-11 14 0 14Q11 14 13 6Q15 -4 5 -8V-14Z"
					fill={p.sclera}
				/>
				<path
					d="M-11 1Q-6 -3 0 1Q6 5 11 1V6Q8 12 0 12Q-8 12 -11 6Z"
					fill={p.cheek}
					stroke="none"
				/>
				<rect x="-6" y="-21" width="12" height="8" rx="2" fill={p.clothing} />
				<circle cx="-3" cy="5" r="2" fill={p.accent} stroke="none" />
				<circle cx="4" cy="-3" r="1.5" fill={p.cheek} stroke="none" />
			</>
		)),
	],
	celebrations: [
		handProp("birthday-cupcake", "Birthday cupcake", (p) => (
			<>
				<path d="M-11 -2H11L8 12H-8Z" fill={p.clothing} />
				<path
					d="M-12 -2Q-16 -8 -9 -10Q-9 -18 0 -15Q9 -18 9 -10Q16 -8 12 -2Z"
					fill={p.cheek}
				/>
				<path d="M0 -15V-23" stroke={p.sclera} strokeWidth="3" />
				<path d="M0 -29Q-7 -22 0 -22Q7 -22 0 -29Z" fill={p.accent} />
				<path d="M-6 2L-4 9M0 2V9M6 2L4 9" stroke={p.accent} />
			</>
		)),
		handProp("festival-fan", "Festival fan", (p) => (
			<>
				<path d="M0 11L-17 -10Q0 -29 17 -10Z" fill={p.cheek} />
				<path d="M0 11L-10 -16M0 11V-20M0 11L10 -16" stroke={p.sclera} />
				<path d="M-17 -10Q0 -24 17 -10" stroke={p.accent} strokeWidth="4" />
				<circle cx="0" cy="9" r="2" fill={p.accent} />
			</>
		)),
		handProp("sparkler", "Sparkler", (p) => (
			<>
				<path d="M-5 13L3 -9" stroke={p.clothing} strokeWidth="2" />
				<path
					d="M3 -22L5 -14L13 -16L8 -10L15 -4L6 -6L3 2L0 -6L-9 -4L-2 -10L-7 -16L1 -14Z"
					fill={p.accent}
				/>
				<path
					d="M-12 -20L-10 -17M15 -22L13 -19M-12 2L-9 0M14 4L12 1"
					stroke={p.cheek}
					strokeWidth="2"
				/>
				<circle cx="3" cy="-10" r="3" fill={p.sclera} stroke="none" />
			</>
		)),
		handProp("wrapped-present", "Wrapped present", (p) => (
			<>
				<rect x="-12" y="-8" width="24" height="22" rx="1" fill={p.clothing} />
				<path d="M-14 -12H14V-5H-14Z" fill={p.cheek} />
				<path d="M-3 -12H3V14H-3Z" fill={p.accent} />
				<path
					d="M0 -12Q-14 -12 -9 -20Q-5 -24 0 -12Q5 -24 9 -20Q14 -12 0 -12Z"
					fill={p.accent}
				/>
			</>
		)),
		handProp("party-popper", "Party popper", (p) => (
			<>
				<path d="M-13 15L-7 -7L8 6Z" fill={p.accent} />
				<path d="M-10 5L-3 11M-8 -2L3 8" stroke={p.cheek} strokeWidth="3" />
				<path
					d="M-3 -12Q-7 -22 -1 -21M6 -8Q16 -16 11 -19M11 -2Q20 -6 16 -10"
					stroke={p.clothing}
					strokeWidth="2"
				/>
				<path
					d="M-10 -18L-13 -15M3 -20L5 -17M16 4L14 7"
					stroke={p.cheek}
					strokeWidth="3"
				/>
			</>
		)),
		handProp("paper-lantern", "Paper lantern", (p) => (
			<>
				<path d="M-7 -16Q-7 -24 0 -24Q7 -24 7 -16" />
				<ellipse cx="0" cy="-3" rx="14" ry="14" fill={p.cheek} />
				<path
					d="M-5 -16Q-12 -3 -5 10M5 -16Q12 -3 5 10M0 -16V10"
					stroke={p.accent}
				/>
				<path d="M-7 -17H7V-13H-7ZM-7 9H7V13H-7Z" fill={p.clothing} />
				<path d="M0 13V18M-3 18H3" stroke={p.accent} strokeWidth="2" />
			</>
		)),
	],
	service: [
		handProp("stop-paddle", "Stop paddle", (p) => (
			<>
				<path d="M0 -2V13" stroke={p.clothing} strokeWidth="4" />
				<path d="M-5 -22H5L12 -15V-5L5 2H-5L-12 -5V-15Z" fill={p.cheek} />
				<path
					d="M-5 -8V-14Q-5 -17 -3 -15V-11V-17Q-1 -19 0 -17V-11V-16Q2 -18 3 -16V-10L5 -12Q8 -13 7 -10L4 -4H-2Z"
					fill={p.sclera}
					stroke="none"
				/>
			</>
		)),
		handProp("rescue-rope-coil", "Rescue rope", (p) => (
			<>
				<ellipse
					cx="0"
					cy="-5"
					rx="12"
					ry="15"
					stroke={p.ink}
					strokeWidth="7"
				/>
				<ellipse
					cx="0"
					cy="-5"
					rx="12"
					ry="15"
					stroke={p.accent}
					strokeWidth="4"
				/>
				<ellipse
					cx="0"
					cy="-5"
					rx="7"
					ry="10"
					stroke={p.clothing}
					strokeWidth="3"
				/>
				<path
					d="M-12 -7L-7 -5M-12 -2L-7 0M7 -7L12 -5M7 -2L12 0M8 8L14 14"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<path d="M-5 -6H5V0H-5Z" fill={p.clothing} />
			</>
		)),
		handProp("fire-extinguisher", "Fire extinguisher", (p) => (
			<>
				<path d="M-2 -15V-21H8M-2 -19H7V-15H-2" strokeWidth="2" />
				<rect x="-8" y="-15" width="16" height="28" rx="5" fill={p.cheek} />
				<path
					d="M-6 -14Q-16 -16 -13 1H-10"
					stroke={p.clothing}
					strokeWidth="3"
				/>
				<path d="M-5 -6H5V5H-5Z" fill={p.sclera} />
				<path d="M0 -3L-3 2H3Z" fill={p.accent} />
			</>
		)),
		handProp(
			"rescue-radio",
			"Rescue radio",
			(p) => (
				<>
					<path d="M-7 -12V-24" strokeWidth="3" />
					<rect
						x="-11"
						y="-13"
						width="22"
						height="28"
						rx="4"
						fill={p.clothing}
					/>
					<rect x="-7" y="-9" width="14" height="8" rx="1" fill={p.accent} />
					<path d="M-6 4H6M-6 8H6" stroke={p.sclera} />
					<path d="M13 -18Q18 -14 13 -10" stroke={p.accent} />
				</>
			),
			"chat",
		),
		handProp("rescue-flashlight", "Flashlight", (p) => (
			<>
				<path d="M-6 -5H6V14H-6Z" fill={p.clothing} />
				<path d="M-10 -18H10V-10L6 -4H-6L-10 -10Z" fill={p.accent} />
				<ellipse cx="0" cy="-18" rx="10" ry="3" fill={p.sclera} />
				<path d="M-3 8H3M-3 11H3M-4 -24L-5 -28M4 -24L5 -28" stroke={p.accent} />
				<rect x="-2" y="0" width="4" height="5" rx="1" fill={p.sclera} />
			</>
		)),
	],
	workplace: [
		handProp("takeaway-coffee", "Takeaway coffee", (p) => (
			<>
				<path d="M-11 -13H11L8 12H-8Z" fill={p.sclera} />
				<path d="M-10 -6H10L9 5H-9Z" fill={p.clothing} />
				<path d="M-13 -14H13V-18H-13ZM-10 -18H10V-21H-10Z" fill={p.accent} />
				<path d="M-3 0Q0 -6 3 0Q0 5 -3 0Z" fill={p.accent} />
			</>
		)),
		handProp(
			"desk-calendar",
			"Desk calendar",
			(p) => (
				<>
					<path d="M-12 -17H11L16 12H-16Z" fill={p.clothing} />
					<path d="M-12 -17H11L8 8H-15Z" fill={p.sclera} />
					<path d="M-12 -17H11L10 -10H-13Z" fill={p.accent} />
					<path d="M-7 -20V-14M4 -20V-14M-9 -5H-6M-2 -5H1M5 -5H7M-10 1H-7M-3 1H0M4 1H6" />
				</>
			),
			"calendar",
		),
		handProp(
			"office-megaphone",
			"Megaphone",
			(p) => (
				<>
					<path d="M-12 -8H-5L10 -18V5L-5 -3H-12Z" fill={p.accent} />
					<path d="M-4 -3L1 -1L-2 11H-7Z" fill={p.clothing} />
					<ellipse cx="10" cy="-6" rx="4" ry="12" fill={p.sclera} />
					<path
						d="M16 -14L19 -17M17 -6H21M16 2L19 5"
						stroke={p.cheek}
						strokeWidth="2"
					/>
				</>
			),
			"chat",
		),
		handProp(
			"work-briefcase",
			"Briefcase",
			(p) => (
				<>
					<path d="M-6 -11V-17H6V-11" strokeWidth="3" />
					<rect
						x="-16"
						y="-11"
						width="32"
						height="24"
						rx="3"
						fill={p.clothing}
					/>
					<path d="M-16 -4Q0 4 16 -4" />
					<rect x="-3" y="-3" width="6" height="6" rx="1" fill={p.accent} />
					<path d="M-11 7V9M11 7V9" stroke={p.sclera} />
				</>
			),
			"files",
		),
		handProp(
			"presentation-clicker",
			"Presentation clicker",
			(p) => (
				<>
					<rect
						x="-7"
						y="-19"
						width="14"
						height="33"
						rx="5"
						fill={p.clothing}
					/>
					<circle cx="0" cy="-9" r="4" fill={p.accent} />
					<path d="M-1 -11L2 -9L-1 -7Z" fill={p.ink} stroke="none" />
					<path d="M-3 1H3M-3 6H3M10 -19L14 -23M11 -13H17" stroke={p.sclera} />
				</>
			),
			"device",
		),
	],
	healthcare: [
		handProp("stethoscope-hand", "Stethoscope", (p) => (
			<>
				<path
					d="M-8 -20V-10Q-8 -2 0 -2Q8 -2 8 -10V-20"
					stroke={p.sclera}
					strokeWidth="3"
				/>
				<path d="M0 -2V5Q0 15 11 10V3" stroke={p.clothing} strokeWidth="3" />
				<path
					d="M-8 -20L-4 -22M8 -20L4 -22"
					stroke={p.clothing}
					strokeWidth="3"
				/>
				<circle cx="11" cy="0" r="5" fill={p.accent} />
				<circle cx="11" cy="0" r="2" fill={p.sclera} />
			</>
		)),
		handProp("reflex-hammer", "Reflex hammer", (p) => (
			<>
				<path d="M-2 -11H2V13H-2Z" fill={p.sclera} />
				<path d="M-3 -12L-12 -20H12L3 -12Z" fill={p.cheek} />
				<path d="M-12 -20H12V-16H-12Z" fill={p.clothing} />
				<path d="M-2 6H2V12H-2Z" fill={p.accent} />
			</>
		)),
		handProp("tooth-model", "Tooth model", (p) => (
			<>
				<path
					d="M0 -19Q-14 -25 -13 -11Q-12 -6 -9 -3L-7 9Q-4 16 -1 4Q0 0 1 4Q4 16 7 9L9 -3Q12 -6 13 -11Q14 -25 0 -19Z"
					fill={p.sclera}
				/>
				<path
					d="M-7 -15Q-4 -18 0 -15Q4 -18 7 -15M0 -15V-9"
					stroke={p.accent}
					strokeWidth="2"
				/>
				<path d="M-9 -4Q0 -1 9 -4" stroke={p.cheek} strokeWidth="2" />
			</>
		)),
		handProp("medical-bag", "Medical bag", (p) => (
			<>
				<path d="M-6 -10V-17H6V-10" strokeWidth="3" />
				<rect x="-15" y="-10" width="30" height="24" rx="5" fill={p.sclera} />
				<path d="M-15 -3H15" stroke={p.clothing} />
				<path d="M-3 -1H3V3H7V9H3V13H-3V9H-7V3H-3Z" fill={p.accent} />
			</>
		)),
		handProp("digital-thermometer", "Thermometer", (p) => (
			<>
				<path d="M-4 -18Q0 -23 4 -18V4L2 14H-2L-4 4Z" fill={p.sclera} />
				<rect x="-3" y="-15" width="6" height="11" rx="1" fill={p.accent} />
				<path d="M-1 -12H1M-1 -9H1M-2 10H2V14H-2Z" fill={p.clothing} />
				<circle cx="0" cy="0" r="1.5" fill={p.cheek} stroke="none" />
			</>
		)),
	],
};
