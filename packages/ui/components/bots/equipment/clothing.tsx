import type { ReactNode } from "react";
import { getBotClothingFit } from "../clothing-fit";
import type { BotEquipmentDefinition, BotPalette } from "../types";

type Neckline = "round" | "v" | "straight" | "accessory";

function outfit(
	id: string,
	label: string,
	details: (palette: BotPalette) => ReactNode,
	neckline: Neckline = "round",
	fabric: keyof BotPalette = "clothing",
): BotEquipmentDefinition {
	return {
		id,
		label,
		slot: "neck",
		render: ({ anchors, palette }) => (
			<g
				transform={getBotClothingFit(anchors).transform}
				stroke={palette.ink}
				strokeWidth="1.15"
				strokeLinecap="round"
				strokeLinejoin="round"
			>
				{neckline !== "accessory" && (
					<path
						d={
							neckline === "v"
								? "M-100 0H-11L0 9L11 0H100V36H-100Z"
								: neckline === "straight"
									? "M-100 0H100V36H-100Z"
									: "M-100 0H-12Q0 11 12 0H100V36H-100Z"
						}
						fill={palette[fabric]}
					/>
				)}
				{details(palette)}
			</g>
		),
	};
}

function Buttons({ color, x = 0 }: { color: string; x?: number }) {
	return (
		<g fill={color} stroke="none">
			<circle cx={x} cy="10" r="1.3" />
			<circle cx={x} cy="16" r="1.3" />
			<circle cx={x} cy="22" r="1.3" />
		</g>
	);
}

function Collar({ color }: { color: string }) {
	return <path d="M-13 0L0 6L-7 12L-18 2ZM13 0L0 6L7 12L18 2Z" fill={color} />;
}

export const clothingBySeries: Readonly<
	Record<string, readonly BotEquipmentDefinition[]>
> = {
	flow: [
		outfit("flow-panel-anorak", "Flow panel anorak", (p) => (
			<>
				<path d="M-100 0H-12L0 7L12 0H100V9L-100 26Z" fill={p.accent} />
				<path
					d="M-13 1Q0 13 13 1M0 7V17"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2.5"
				/>
				<path d="M-17 19H17V28H-17ZM-6 19V28" fill={p.clothing} />
				<path d="M-23 6V15M23 6V15" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		outfit("connector-tabard", "Connector tabard", (p) => (
			<>
				<path d="M-21 0H-13V8H13V0H21V32H-21Z" fill={p.sclera} />
				<path
					d="M-13 17H0V25H13"
					fill="none"
					stroke={p.clothing}
					strokeWidth="2.5"
				/>
				<rect x="-16" y="13" width="7" height="7" rx="2" fill={p.accent} />
				<rect x="-3" y="13" width="7" height="7" rx="2" fill={p.clothing} />
				<rect x="10" y="21" width="7" height="7" rx="2" fill={p.accent} />
			</>
		)),
		outfit("flow-athletics-singlet", "Flow athletics singlet", (p) => (
			<>
				<path
					d="M-25 0V10Q-25 18 -35 20M25 0V10Q25 18 35 20M-12 0Q0 15 12 0"
					fill="none"
					stroke={p.sclera}
					strokeWidth="3"
				/>
				<path
					d="M-8 12H8V16H1L-3 29H-8L-3 17H-8Z"
					fill={p.accent}
					stroke="none"
				/>
				<path
					d="M-100 27L-18 20M100 27L18 20"
					stroke={p.accent}
					strokeWidth="4"
				/>
			</>
		)),
		outfit("flow-track-jacket", "Flow track jacket", (p) => (
			<>
				<path
					d="M-12 0Q0 11 12 0M0 6V30"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2.5"
				/>
				<path d="M-28 1L-18 27M28 1L18 27" stroke={p.accent} strokeWidth="4" />
				<path
					d="M9 9H18L13 14H21L10 22L13 16H7Z"
					fill={p.sclera}
					stroke="none"
				/>
			</>
		)),
		outfit("flow-crew-shirt", "Flow crew shirt", (p) => (
			<>
				<path
					d="M-40 15Q-20 5 0 15T40 15V30H-40Z"
					fill={p.accent}
					stroke="none"
				/>
				<path
					d="M-11 13H-3V20H5V13H13M-12 2Q0 12 12 2"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2.5"
				/>
			</>
		)),
	],
	basics: [
		outfit(
			"moon-pajamas",
			"Moon pajamas",
			(p) => (
				<>
					<path
						d="M-16 0L-21 7L-7 12L0 24L8 12L21 7L16 0M0 24V32"
						fill="none"
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<path d="M14 15H26V25H14Z" fill="none" stroke={p.sclera} />
					<path
						d="M-20 13Q-10 14 -12 23Q-22 26 -24 17Q-21 20 -18 18Q-15 16 -20 13Z"
						fill={p.accent}
						stroke="none"
					/>
					<circle cx="3" cy="26" r="1.5" fill={p.accent} />
				</>
			),
			"v",
		),
		outfit("bomber-jacket", "Bomber jacket", (p) => (
			<>
				<path d="M-13 0Q0 12 13 0L10 6Q0 16 -10 6Z" fill={p.accent} />
				<path
					d="M0 10V32M-24 14L-15 23M24 14L15 23"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<path d="M-100 27H100" stroke={p.accent} strokeWidth="5" />
				<rect x="-2" y="11" width="4" height="5" rx="1" fill={p.sclera} />
				<path d="M-30 5H-22V14H-30Z" fill={p.accent} />
			</>
		)),
		outfit(
			"wrap-cardigan",
			"Wrap cardigan",
			(p) => (
				<>
					<path
						d="M-15 0L18 32H-100V0ZM15 0L-14 32"
						fill={p.clothing}
						stroke={p.sclera}
						strokeWidth="2.5"
					/>
					<path d="M-100 23H100" stroke={p.accent} strokeWidth="5" />
					<path
						d="M14 21Q4 12 6 23L14 25Q24 15 25 23L14 25L11 32M16 25L23 32"
						fill={p.accent}
					/>
					<path
						d="M-27 12L-19 19M-31 16L-23 23"
						stroke={p.sclera}
						opacity=".55"
					/>
				</>
			),
			"v",
		),
		outfit(
			"cardigan",
			"Cardigan",
			(p) => (
				<>
					<path
						d="M-11 0L0 11L11 0M0 11V32"
						fill="none"
						stroke={p.sclera}
						strokeWidth="3"
					/>
					<path d="M-25 14H-13V23H-25ZM13 14H25V23H13Z" fill="none" />
					<Buttons color={p.accent} x={0} />
				</>
			),
			"v",
		),
		outfit("denim-jacket", "Denim jacket", (p) => (
			<>
				<Collar color={p.accent} />
				<path
					d="M0 7V32M-28 10H-13V19L-20 23L-28 19ZM13 10H28V19L20 23L13 19Z"
					fill="none"
					stroke={p.sclera}
				/>
				<path d="M-28 10L-20 14L-13 10M13 10L20 14L28 10" fill="none" />
				<Buttons color={p.accent} />
			</>
		)),
		outfit("varsity-jacket", "Varsity jacket", (p) => (
			<>
				<path d="M-100 0H-25L-19 32H-100ZM25 0H100V32H19Z" fill={p.sclera} />
				<path
					d="M-11 1Q0 10 11 1M0 7V32"
					fill="none"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path
					d="M9 10L12 21L16 10L20 21L23 10"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2.5"
				/>
				<Buttons color={p.accent} x={-3} />
			</>
		)),
		outfit("quilted-vest", "Quilted vest", (p) => (
			<>
				<path
					d="M-28 0L-22 32M28 0L22 32M0 6V32"
					fill="none"
					stroke={p.accent}
					strokeWidth="2"
				/>
				<path
					d="M-40 8H-3M3 8H40M-40 15H-3M3 15H40M-40 22H-3M3 22H40"
					fill="none"
					stroke={p.sclera}
					opacity=".7"
				/>
				<rect x="-2" y="8" width="4" height="5" rx="1" fill={p.sclera} />
			</>
		)),
		outfit("striped-shirt", "Striped shirt", (p) => (
			<>
				<path
					d="M-100 11H100M-100 20H100M-100 29H100"
					stroke={p.sclera}
					strokeWidth="4"
				/>
				<path
					d="M-12 1Q0 12 12 1"
					stroke={p.accent}
					strokeWidth="3"
					fill="none"
				/>
				<path d="M13 8H24V16Q18 20 13 16Z" fill={p.clothing} />
			</>
		)),
		outfit("polo-shirt", "Polo shirt", (p) => (
			<>
				<Collar color={p.sclera} />
				<path d="M-2 7V17H2V7" fill={p.accent} />
				<path d="M18 10L22 14L18 18L14 14Z" fill={p.accent} />
			</>
		)),
		outfit(
			"bib-overalls",
			"Bib overalls",
			(p) => (
				<>
					<path
						d="M-22 0H-15V12H15V0H22V14L30 32H-30L-22 14Z"
						fill={p.clothing}
					/>
					<path d="M-12 14H12V23Q0 29 -12 23Z" fill="none" stroke={p.sclera} />
					<circle cx="-18" cy="10" r="2" fill={p.accent} />
					<circle cx="18" cy="10" r="2" fill={p.accent} />
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"raincoat",
			"Raincoat",
			(p) => (
				<>
					<path
						d="M-18 0L-10 9L0 5L10 9L18 0M0 5V32"
						fill="none"
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<path
						d="M-26 15L-12 12V19L-26 22ZM12 12L26 15V22L12 19Z"
						fill={p.clothing}
					/>
					<Buttons color={p.clothing} x={4} />
				</>
			),
			"round",
			"accent",
		),
	],
	nature: [
		outfit("rain-cloud-poncho", "Rain cloud poncho", (p) => (
			<>
				<path
					d="M-28 0Q-25 10 -18 6Q-11 14 -5 8Q1 16 7 8Q15 14 21 6Q28 9 30 0"
					fill={p.sclera}
				/>
				<path
					d="M-19 13Q-26 21 -19 22Q-12 21 -19 13ZM0 15Q-7 23 0 24Q7 23 0 15ZM19 13Q12 21 19 22Q26 21 19 13Z"
					fill={p.accent}
					stroke="none"
				/>
				<path
					d="M-100 29Q-25 21 0 29T100 29"
					stroke={p.sclera}
					fill="none"
					strokeWidth="2"
				/>
			</>
		)),
		outfit(
			"bark-waistcoat",
			"Bark waistcoat",
			(p) => (
				<>
					<path d="M-11 0L-5 32H5L11 0" fill={p.accent} />
					<path
						d="M-25 2L-21 10L-26 17L-23 29M-16 8L-14 17L-18 24M24 1L20 8L25 15L21 29M15 12L18 18L15 27"
						fill="none"
						stroke={p.sclera}
						opacity=".65"
					/>
					<path d="M-6 12H6M-5 22H5" stroke={p.ink} strokeWidth="3" />
					<ellipse cx="0" cy="12" rx="4" ry="1.8" fill={p.sclera} />
					<ellipse cx="0" cy="22" rx="4" ry="1.8" fill={p.sclera} />
				</>
			),
			"v",
		),
		outfit(
			"meadow-blouse",
			"Meadow blouse",
			(p) => (
				<>
					<path
						d="M-31 0Q-20 10 -12 2M31 0Q20 10 12 2M-12 1Q0 14 12 1"
						fill="none"
						stroke={p.accent}
						strokeWidth="3"
					/>
					<path
						d="M-20 24V14M-20 20L-25 17M-20 18L-15 15M18 25V13M18 20L24 17"
						stroke={p.clothing}
						fill="none"
						strokeWidth="1.5"
					/>
					<path
						d="M-20 10C-29 5 -29 17 -20 14C-12 20 -11 8 -20 10ZM18 10C10 4 8 17 17 14C25 21 29 7 18 10Z"
						fill={p.cheek}
					/>
					<circle cx="-20" cy="12" r="2" fill={p.accent} />
					<circle cx="18" cy="12" r="2" fill={p.accent} />
					<path d="M0 9V26" stroke={p.clothing} strokeDasharray="2 3" />
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"leaf-mantle",
			"Leaf mantle",
			(p) => (
				<>
					<path
						d="M-28 1Q-26 17 -13 21L-5 8Q0 23 9 24L14 8Q27 17 29 1L12 0Q0 10 -12 0Z"
						fill={p.clothing}
					/>
					<path
						d="M-23 3L-14 16M0 8L7 19M23 3L17 10"
						fill="none"
						stroke={p.accent}
					/>
					<circle cx="0" cy="8" r="3" fill={p.accent} />
				</>
			),
			"accessory",
		),
		outfit(
			"woodland-tunic",
			"Woodland tunic",
			(p) => (
				<>
					<path
						d="M-11 0L0 10L11 0M-4 6L4 10M-4 10L4 14M-4 14L4 18"
						fill="none"
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<path d="M-100 24H100" stroke={p.accent} strokeWidth="5" />
					<path d="M17 6Q30 9 19 19Q13 15 17 6Z" fill={p.accent} />
				</>
			),
			"v",
		),
	],
	pets: [
		outfit(
			"pet-rain-cape",
			"Pet rain cape",
			(p) => (
				<>
					<path
						d="M-31 0L-10 6L-18 32H-100V0ZM31 0L10 6L18 32H100V0Z"
						fill={p.accent}
					/>
					<path
						d="M-29 2Q0 21 29 2M-19 16L-26 28M19 16L26 28"
						fill="none"
						stroke={p.sclera}
						strokeWidth="2.5"
					/>
					<rect x="-9" y="7" width="18" height="7" rx="3" fill={p.clothing} />
					<circle cx="0" cy="10.5" r="1.5" fill={p.sclera} />
				</>
			),
			"accessory",
		),
		outfit(
			"pet-training-harness",
			"Pet training harness",
			(p) => (
				<>
					<path
						d="M-20 0Q-8 13 -19 29M20 0Q8 13 19 29M-100 24H100"
						stroke={p.clothing}
						fill="none"
						strokeWidth="7"
					/>
					<path d="M-12 12H12V23H-12Z" fill={p.accent} />
					<path
						d="M-5 14Q-10 10 -10 15Q-13 19 -7 19H7Q13 19 10 15Q10 10 5 14Z"
						fill={p.sclera}
					/>
					<path d="M-15 8H-9M9 8H15" stroke={p.sclera} strokeWidth="2" />
				</>
			),
			"accessory",
		),
		outfit("cozy-pet-sweater", "Cozy pet sweater", (p) => (
			<>
				<path
					d="M-15 0Q0 10 15 0M-100 24H100"
					fill="none"
					stroke={p.accent}
					strokeWidth="5"
				/>
				<path
					d="M-26 7V21M-21 9V21M21 9V21M26 7V21"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<path
					d="M-7 16Q0 7 7 16Q10 23 0 21Q-10 23 -7 16Z"
					fill={p.cheek}
					stroke="none"
				/>
				<g fill={p.cheek} stroke="none">
					<circle cx="-8" cy="10" r="2.5" />
					<circle cx="0" cy="7" r="2.5" />
					<circle cx="8" cy="10" r="2.5" />
				</g>
			</>
		)),
		outfit(
			"paw-bandana",
			"Paw bandana",
			(p) => (
				<>
					<path d="M-28 0Q0 9 28 0L0 25Z" fill={p.clothing} />
					<g fill={p.accent} stroke="none">
						<ellipse cx="0" cy="15" rx="4" ry="3" />
						<circle cx="-5" cy="11" r="1.7" />
						<circle cx="0" cy="9" r="1.7" />
						<circle cx="5" cy="11" r="1.7" />
					</g>
				</>
			),
			"accessory",
		),
		outfit(
			"bell-collar",
			"Bell collar",
			(p) => (
				<>
					<path d="M-100 0Q0 12 100 0V7Q0 20 -100 7Z" fill={p.clothing} />
					<circle cx="0" cy="12" r="2.5" fill={p.accent} />
					<circle cx="0" cy="19" r="6" fill={p.accent} />
					<path d="M-4 20H4M0 20V25" fill="none" />
				</>
			),
			"accessory",
		),
	],
	zoo: [
		outfit(
			"giraffe-patch-vest",
			"Giraffe patch vest",
			(p) => (
				<>
					<path
						d="M-30 2L-18 1L-14 9L-24 13L-31 9ZM16 2L28 3L30 12L19 14L13 8ZM-28 17L-16 13L-10 23L-19 30L-29 26ZM16 18L28 17L31 27L22 31L13 26Z"
						fill={p.clothing}
						stroke="none"
					/>
					<path
						d="M-11 0L0 12L11 0M0 12V32"
						stroke={p.sclera}
						fill="none"
						strokeWidth="2.5"
					/>
					<Buttons color={p.ink} />
				</>
			),
			"v",
			"accent",
		),
		outfit(
			"penguin-dinner-bib",
			"Penguin dinner bib",
			(p) => (
				<>
					<path d="M-17 0Q-28 16 -17 32H17Q28 16 17 0L0 5Z" fill={p.sclera} />
					<path d="M-10 1L0 7L10 1V12L0 9L-10 12Z" fill={p.accent} />
					<circle cx="0" cy="16" r="1.5" fill={p.clothing} />
					<circle cx="0" cy="23" r="1.5" fill={p.clothing} />
					<path
						d="M-31 7L-20 24M31 7L20 24"
						stroke={p.accent}
						strokeWidth="2"
					/>
				</>
			),
			"straight",
		),
		outfit("tortoise-shell-vest", "Tortoise shell vest", (p) => (
			<>
				<path
					d="M-13 0L-6 9H6L13 0M-6 9L-13 19L-6 30H6L13 19L6 9M-13 19H-29L-35 8M13 19H29L35 8M-6 30L-20 34M6 30L20 34"
					stroke={p.accent}
					strokeWidth="2.5"
					fill="none"
				/>
				<path d="M-6 12H6L10 19L5 26H-5L-10 19Z" fill={p.accent} />
				<path d="M-24 6L-18 10M24 6L18 10" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		outfit(
			"safari-vest",
			"Safari vest",
			(p) => (
				<>
					<path d="M-9 0L-3 32H3L9 0" fill={p.sclera} />
					<path d="M-28 6H-13V17H-28ZM13 6H28V17H13Z" fill={p.accent} />
					<path
						d="M-28 6L-20 11L-13 6M13 6L20 11L28 6M-24 22H-12M12 22H24"
						fill="none"
					/>
				</>
			),
			"v",
		),
		outfit(
			"zebra-shirt",
			"Zebra shirt",
			(p) => (
				<>
					<path
						d="M-35 0L-18 13L-30 10L-18 27L-37 15ZM35 0L18 13L30 10L18 27L37 15ZM-9 7L2 15L-5 23L10 32L0 16L9 9Z"
						fill={p.ink}
						stroke="none"
					/>
					<path
						d="M-12 1Q0 11 12 1"
						fill="none"
						stroke={p.accent}
						strokeWidth="3"
					/>
				</>
			),
			"round",
			"sclera",
		),
	],
	tech: [
		outfit("control-console-vest", "Control console vest", (p) => (
			<>
				<path d="M-24 0L-19 32M24 0L19 32" stroke={p.accent} strokeWidth="5" />
				<rect x="-14" y="8" width="28" height="19" rx="4" fill={p.ink} />
				<path d="M-10 12H4V20H-10Z" fill={p.sclera} />
				<path d="M-8 17L-5 14L-2 18L2 15" stroke={p.accent} fill="none" />
				<circle cx="9" cy="13" r="2" fill={p.cheek} />
				<circle cx="9" cy="20" r="2" fill={p.accent} />
				<path d="M-7 24H4" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		outfit(
			"fiber-optic-mantle",
			"Fiber optic mantle",
			(p) => (
				<>
					<path
						d="M-29 0Q0 14 29 0L24 28L12 22L0 30L-12 22L-24 28Z"
						fill={p.clothing}
					/>
					<path
						d="M-20 3L-21 23M-10 6L-11 19M0 8V26M10 6L11 19M20 3L21 23"
						stroke={p.accent}
						fill="none"
						strokeWidth="2"
					/>
					<g fill={p.sclera} stroke="none">
						<circle cx="-21" cy="23" r="2" />
						<circle cx="-11" cy="19" r="2" />
						<circle cx="0" cy="26" r="2" />
						<circle cx="11" cy="19" r="2" />
						<circle cx="21" cy="23" r="2" />
					</g>
				</>
			),
			"accessory",
		),
		outfit("robot-chest-plate", "Robot chest plate", (p) => (
			<>
				<path
					d="M-30 0H-17L-12 7H12L17 0H30L26 26L14 30H-14L-26 26Z"
					fill={p.sclera}
				/>
				<path d="M-7 10H7L11 17L7 24H-7L-11 17Z" fill={p.accent} />
				<path
					d="M-23 10V23M23 10V23M-26 14H-19M19 14H26"
					stroke={p.clothing}
					strokeWidth="2"
				/>
				<circle cx="0" cy="17" r="3.5" fill={p.clothing} />
			</>
		)),
		outfit("circuit-jacket", "Circuit jacket", (p) => (
			<>
				<path
					d="M0 6V32M-25 4V13H-14V24M25 4V10H14V19H25"
					fill="none"
					stroke={p.accent}
					strokeWidth="2"
				/>
				<g fill={p.sclera}>
					<circle cx="-14" cy="24" r="2" />
					<circle cx="25" cy="19" r="2" />
					<circle cx="-25" cy="4" r="2" />
				</g>
				<path d="M-6 5H6V12H-6Z" fill={p.accent} />
			</>
		)),
		outfit(
			"utility-harness",
			"Utility harness",
			(p) => (
				<>
					<path
						d="M-20 0L-8 29M20 0L8 29M-30 19H30"
						fill="none"
						stroke={p.clothing}
						strokeWidth="6"
					/>
					<rect x="-8" y="13" width="16" height="13" rx="3" fill={p.accent} />
					<path d="M-4 17H4V22H-4ZM17 10H25V22H17Z" fill={p.sclera} />
				</>
			),
			"accessory",
		),
	],
	snacks: [
		outfit(
			"diner-uniform",
			"Diner uniform",
			(p) => (
				<>
					<Collar color={p.sclera} />
					<path d="M-100 21H100" stroke={p.sclera} strokeWidth="6" />
					<path
						d="M-12 0L2 14V32"
						stroke={p.sclera}
						strokeWidth="2"
						fill="none"
					/>
					<ellipse cx="19" cy="11" rx="8" ry="4" fill={p.sclera} />
					<path d="M15 11H23" stroke={p.clothing} />
					<path d="M-7 22L2 25L10 21L9 29L2 26L-6 30Z" fill={p.accent} />
				</>
			),
			"round",
			"cheek",
		),
		outfit(
			"baker-striped-smock",
			"Baker striped smock",
			(p) => (
				<>
					<path
						d="M-28 0V32M-18 0V32M-8 7V32M2 8V32M12 3V32M22 0V32"
						stroke={p.clothing}
						strokeWidth="3"
					/>
					<path
						d="M-14 0Q0 13 14 0M-30 27H30"
						stroke={p.accent}
						strokeWidth="3"
						fill="none"
					/>
					<path d="M-12 13H12V24Q0 30 -12 24Z" fill={p.sclera} />
					<path d="M-4 18Q0 12 4 18L6 23H-6Z" fill={p.accent} />
					<path d="M-2 17L0 20M1 16L3 19" stroke={p.clothing} />
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"sushi-wrap-tunic",
			"Sushi wrap tunic",
			(p) => (
				<>
					<path
						d="M-14 0L15 32M14 0L-4 21"
						stroke={p.sclera}
						strokeWidth="5"
						fill="none"
					/>
					<path d="M-100 25H100" stroke={p.accent} strokeWidth="7" />
					<path
						d="M-25 13Q-16 6 -11 13Q-16 20 -25 13L-29 8V18Z"
						fill={p.cheek}
					/>
					<circle cx="-15" cy="12" r="1" fill={p.ink} />
					<path d="M18 7L25 14M21 4L28 11" stroke={p.accent} strokeWidth="2" />
				</>
			),
			"v",
		),
		outfit(
			"chef-jacket",
			"Chef jacket",
			(p) => (
				<>
					<Collar color={p.sclera} />
					<path d="M7 7V32M-26 12H-15V19H-26" fill="none" stroke={p.clothing} />
					<Buttons color={p.clothing} x={-5} />
					<Buttons color={p.clothing} x={6} />
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"barista-apron",
			"Barista apron",
			(p) => (
				<>
					<path
						d="M-15 0V7H15V0M-15 7L-28 32H28L15 7Z"
						fill={p.clothing}
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path d="M-7 13H5V18Q-1 23 -7 18ZM5 14H9V17H5" fill={p.sclera} />
					<path d="M-15 25H15" stroke={p.accent} />
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"picnic-bib",
			"Picnic bib",
			(p) => (
				<>
					<path d="M-19 0Q0 12 19 0L23 21Q0 34 -23 21Z" fill={p.sclera} />
					<path
						d="M-16 10H16M-18 18H18M-10 6V24M0 7V27M10 6V24"
						stroke={p.clothing}
						strokeWidth="3"
						opacity=".65"
					/>
					<path d="M19 2L29 1L25 9L19 6Z" fill={p.accent} />
				</>
			),
			"accessory",
		),
	],
	cosmos: [
		outfit("mission-control-jacket", "Mission control jacket", (p) => (
			<>
				<path
					d="M-12 0L-9 7H9L12 0M0 7V32"
					stroke={p.sclera}
					strokeWidth="2"
					fill="none"
				/>
				<rect x="-28" y="9" width="17" height="7" rx="1" fill={p.sclera} />
				<path d="M-25 12H-16" stroke={p.clothing} />
				<path d="M13 8H28V21H13Z" fill={p.accent} />
				<path d="M16 17L19 11L22 17L25 13" stroke={p.clothing} fill="none" />
				<path d="M-100 25H100" stroke={p.sclera} strokeWidth="3" />
			</>
		)),
		outfit(
			"meteor-poncho",
			"Meteor poncho",
			(p) => (
				<>
					<path
						d="M-100 0H-12L0 7L12 0H100L30 27L19 21L9 30L0 23L-9 30L-19 21L-30 27Z"
						fill={p.clothing}
					/>
					<path
						d="M-25 7L-13 13M-20 3L-9 9M13 18L25 25"
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<path
						d="M-10 10L-4 8L1 13L-2 20L-9 19L-13 15ZM16 17L22 14L27 19L24 24L17 23Z"
						fill={p.accent}
					/>
					<path d="M8 4V10M5 7H11" stroke={p.accent} strokeWidth="1.5" />
				</>
			),
			"accessory",
		),
		outfit(
			"constellation-cape",
			"Constellation cape",
			(p) => (
				<>
					<path
						d="M-31 0L-8 7L-15 32H-100V0ZM31 0L8 7L15 32H100V0Z"
						fill={p.clothing}
					/>
					<path
						d="M-25 7L-16 14L-26 25M22 7L15 17L28 23M-7 8Q0 15 7 8"
						stroke={p.accent}
						fill="none"
					/>
					<g fill={p.sclera} stroke="none">
						<circle cx="-25" cy="7" r="2" />
						<circle cx="-16" cy="14" r="2" />
						<circle cx="-26" cy="25" r="2" />
						<circle cx="22" cy="7" r="2" />
						<circle cx="15" cy="17" r="2" />
						<circle cx="28" cy="23" r="2" />
					</g>
					<path d="M0 7L3 10L0 14L-3 10Z" fill={p.accent} />
				</>
			),
			"accessory",
		),
		outfit("flight-suit", "Flight suit", (p) => (
			<>
				<path
					d="M-11 0L-5 8H5L11 0M0 8V32"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<path d="M-26 9H-13V18H-26Z" fill={p.accent} />
				<path d="M14 9L20 6L26 9L20 19Z" fill={p.sclera} />
				<path d="M-27 23H27" stroke={p.ink} strokeWidth="3" />
			</>
		)),
		outfit(
			"lunar-suit",
			"Lunar suit",
			(p) => (
				<>
					<path d="M-30 0Q0 13 30 0L26 7Q0 19 -26 7Z" fill={p.accent} />
					<rect
						x="-13"
						y="12"
						width="26"
						height="15"
						rx="3"
						fill={p.clothing}
					/>
					<path
						d="M-8 17H2M-8 21H0M17 13Q29 16 23 30"
						fill="none"
						stroke={p.accent}
						strokeWidth="2.5"
					/>
					<circle cx="7" cy="18" r="2" fill={p.sclera} />
				</>
			),
			"straight",
			"sclera",
		),
	],
	pro: [
		outfit(
			"shawl-dinner-jacket",
			"Shawl dinner jacket",
			(p) => (
				<>
					<path d="M-12 0H12L0 30Z" fill={p.sclera} />
					<path
						d="M-18 0Q-24 11 -5 26L0 31L-9 7ZM18 0Q24 11 5 26L0 31L9 7Z"
						fill={p.accent}
					/>
					<path d="M-5 4H5L3 9L5 19L0 23L-5 19L-3 9Z" fill={p.clothing} />
					<path
						d="M16 14H27M17 13L19 8L22 12L26 9V14"
						stroke={p.sclera}
						fill="none"
					/>
				</>
			),
			"straight",
		),
		outfit(
			"cravat-waistcoat",
			"Cravat waistcoat",
			(p) => (
				<>
					<path d="M-14 0L-7 32H7L14 0" fill={p.sclera} />
					<path
						d="M-8 1Q0 8 8 1L5 11L10 22L0 29L-10 22L-5 11Z"
						fill={p.accent}
					/>
					<path d="M-5 11H5M0 13L-4 23M0 13L4 23" fill="none" />
					<path
						d="M-26 16H-15M15 16H26M15 18Q19 28 28 21"
						stroke={p.sclera}
						fill="none"
					/>
					<circle cx="0" cy="10" r="2" fill={p.sclera} />
				</>
			),
			"v",
		),
		outfit(
			"herringbone-coat",
			"Herringbone coat",
			(p) => (
				<>
					<path
						d="M-30 8L-24 3L-18 8M-30 17L-24 12L-18 17M-30 26L-24 21L-18 26M18 8L24 3L30 8M18 17L24 12L30 17M18 26L24 21L30 26"
						stroke={p.sclera}
						opacity=".7"
						fill="none"
						strokeWidth="2"
					/>
					<path
						d="M-15 0L-18 7L-10 10L0 29L10 10L18 7L15 0"
						fill={p.clothing}
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path
						d="M-11 22L-18 27M11 22L18 27"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<circle cx="0" cy="27" r="2" fill={p.accent} />
				</>
			),
			"v",
		),
		outfit(
			"tuxedo",
			"Tuxedo",
			(p) => (
				<>
					<path d="M-13 0H13L0 29Z" fill={p.sclera} />
					<path
						d="M-17 0L-22 8L-10 11L0 29L10 11L22 8L17 0"
						fill="none"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path d="M-8 5L0 8L8 5V13L0 10L-8 13Z" fill={p.ink} />
					<path d="M18 14L22 10L26 14Z" fill={p.sclera} />
				</>
			),
			"straight",
		),
		outfit(
			"double-breasted-waistcoat",
			"Double-breasted waistcoat",
			(p) => (
				<>
					<path
						d="M-12 0L0 13L12 0M0 13L9 32M-26 20L-14 18M14 18L26 20"
						fill="none"
						stroke={p.sclera}
						strokeWidth="1.8"
					/>
					<Buttons color={p.accent} x={-5} />
					<Buttons color={p.accent} x={5} />
					<path d="M14 21Q19 29 25 21" fill="none" stroke={p.accent} />
				</>
			),
			"v",
		),
	],
	office: [
		outfit(
			"argyle-slipover",
			"Argyle slipover",
			(p) => (
				<>
					<path
						d="M-12 0L0 11L12 0"
						stroke={p.sclera}
						strokeWidth="3"
						fill="none"
					/>
					<path
						d="M-22 10L-13 19L-22 28L-31 19ZM0 12L9 21L0 30L-9 21ZM22 10L31 19L22 28L13 19Z"
						fill={p.accent}
						stroke="none"
					/>
					<path
						d="M-33 11L-11 31M-33 29L-11 9M-11 13L11 33M-11 31L11 11M11 11L33 31M11 29L33 9"
						stroke={p.sclera}
						fill="none"
					/>
				</>
			),
			"v",
		),
		outfit(
			"loose-necktie-shirt",
			"Loose necktie shirt",
			(p) => (
				<>
					<path
						d="M-16 0L-6 2L-3 12L-13 8ZM16 0L6 2L3 12L13 8Z"
						fill={p.sclera}
					/>
					<path d="M-5 11H4L2 16L7 27L1 31L-4 27L-1 16Z" fill={p.clothing} />
					<path
						d="M-100 23H-20M20 23H100"
						stroke={p.clothing}
						strokeWidth="4"
					/>
					<path d="M15 12H27V20L21 23L15 20Z" fill="none" stroke={p.clothing} />
					<path d="M0 5V9" stroke={p.clothing} />
				</>
			),
			"v",
			"sclera",
		),
		outfit(
			"suspenders-shirt",
			"Suspenders shirt",
			(p) => (
				<>
					<Collar color={p.sclera} />
					<path
						d="M-23 0L-16 32M23 0L16 32"
						stroke={p.clothing}
						strokeWidth="5"
					/>
					<path d="M-23 15H-16V22H-23ZM16 15H23V22H16Z" fill={p.accent} />
					<path d="M0 7V32" stroke={p.clothing} />
					<Buttons color={p.clothing} />
					<path d="M-10 5L0 8L10 5V12L0 10L-10 12Z" fill={p.accent} />
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"cable-knit-vest",
			"Cable-knit vest",
			(p) => (
				<>
					<path
						d="M-12 0L0 11L12 0"
						fill="none"
						stroke={p.accent}
						strokeWidth="3"
					/>
					<path
						d="M-22 3Q-12 9 -22 15T-22 28M-17 3Q-27 9 -17 15T-17 28M17 3Q27 9 17 15T17 28M22 3Q12 9 22 15T22 28M-6 16V29M0 16V29M6 16V29"
						fill="none"
						stroke={p.sclera}
					/>
				</>
			),
			"v",
		),
		outfit(
			"pencil-pocket-shirt",
			"Pencil pocket shirt",
			(p) => (
				<>
					<Collar color={p.clothing} />
					<path d="M0 7V32" stroke={p.clothing} />
					<Buttons color={p.clothing} />
					<path d="M16 7V17M21 5V17" stroke={p.accent} strokeWidth="3" />
					<path d="M12 13H26V23Q19 27 12 23Z" fill={p.sclera} />
				</>
			),
			"round",
			"sclera",
		),
	],
	workshop: [
		outfit(
			"pottery-smock",
			"Pottery smock",
			(p) => (
				<>
					<path
						d="M-18 0L-13 8H13L18 0M-28 22H-11M11 22H28"
						stroke={p.accent}
						strokeWidth="3"
						fill="none"
					/>
					<path d="M-16 13H16V27H-16Z" fill={p.sclera} />
					<path
						d="M-4 15Q1 10 4 16L8 19L4 22Q0 29 -4 22L-9 21Z"
						fill={p.accent}
						stroke="none"
					/>
					<circle cx="-25" cy="10" r="3" fill={p.accent} stroke="none" />
					<path d="M23 10L27 15L21 17Z" fill={p.cheek} stroke="none" />
				</>
			),
			"straight",
		),
		outfit(
			"carpenter-tool-belt",
			"Carpenter tool belt",
			(p) => (
				<>
					<path d="M-100 5H100V13H-100Z" fill={p.clothing} />
					<rect x="-6" y="4" width="12" height="11" rx="2" fill={p.accent} />
					<path d="M-2 7H3V12H-2Z" fill={p.clothing} />
					<path d="M-28 11H-12V29H-28ZM12 11H28V29H12Z" fill={p.accent} />
					<path
						d="M-23 8V23M-17 8V23M20 9V24"
						stroke={p.sclera}
						strokeWidth="3"
					/>
					<path d="M14 7H26V12H14Z" fill={p.sclera} />
					<path d="M-28 21H-12M12 20H28" stroke={p.clothing} />
				</>
			),
			"accessory",
		),
		outfit("patchwork-work-coat", "Patchwork work coat", (p) => (
			<>
				<Collar color={p.accent} />
				<path d="M-28 7H-14V19H-28ZM13 14H29V29H13Z" fill={p.sclera} />
				<path d="M-13 23H-2V32H-13Z" fill={p.cheek} />
				<path
					d="M-26 6V9M-21 6V9M-16 6V9M-29 13H-26M-29 18H-26M14 13V16M20 13V16M26 13V16M12 20H15M12 26H15"
					stroke={p.accent}
					strokeWidth="1.5"
				/>
				<path d="M2 8V32" stroke={p.sclera} />
				<Buttons color={p.accent} x={4} />
			</>
		)),
		outfit(
			"maker-apron",
			"Maker apron",
			(p) => (
				<>
					<path
						d="M-13 0V7H13V0M-13 7L-28 32H28L13 7Z"
						fill={p.clothing}
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path
						d="M-18 16H18V27H-18ZM-6 16V27M6 16V27"
						fill="none"
						stroke={p.sclera}
					/>
					<path
						d="M-12 12V21M0 13V22M11 9V21"
						stroke={p.accent}
						strokeWidth="3"
					/>
				</>
			),
			"round",
			"sclera",
		),
		outfit("welding-jacket", "Welding jacket", (p) => (
			<>
				<path
					d="M-13 0L-9 6H9L13 0M5 6V32M-100 19H100"
					fill="none"
					stroke={p.accent}
					strokeWidth="2"
				/>
				<path d="M-28 7H-13V15H-28ZM13 7H28V15H13Z" fill={p.accent} />
				<path d="M-25 10H-16M16 10H25" stroke={p.ink} />
				<Buttons color={p.sclera} x={2} />
			</>
		)),
	],
	ocean: [
		outfit("fisher-oilskin-bib", "Fisher oilskin bib", (p) => (
			<>
				<path d="M-22 0H-15V10H15V0H22V13L30 32H-30L-22 13Z" fill={p.accent} />
				<path d="M-13 15H13V26H-13ZM0 15V26" fill="none" stroke={p.clothing} />
				<rect x="-23" y="6" width="9" height="6" rx="1" fill={p.sclera} />
				<rect x="14" y="6" width="9" height="6" rx="1" fill={p.sclera} />
				<path
					d="M-7 19L-3 22L-7 25M7 19L3 22L7 25"
					fill="none"
					stroke={p.sclera}
				/>
			</>
		)),
		outfit("lifeguard-singlet", "Lifeguard singlet", (p) => (
			<>
				<path
					d="M-26 0V9Q-26 18 -35 21M26 0V9Q26 18 35 21M-12 0Q0 15 12 0"
					stroke={p.sclera}
					strokeWidth="3"
					fill="none"
				/>
				<circle cx="0" cy="19" r="8" fill={p.sclera} />
				<circle cx="0" cy="19" r="4" fill={p.clothing} />
				<path
					d="M-6 13L-3 16M6 13L3 16M-6 25L-3 22M6 25L3 22"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path d="M-100 28H-16M16 28H100" stroke={p.accent} strokeWidth="3" />
			</>
		)),
		outfit(
			"shell-scale-collar",
			"Shell scale collar",
			(p) => (
				<>
					<path
						d="M-29 0Q0 12 29 0L26 17Q18 28 10 21Q0 33 -10 21Q-18 28 -26 17Z"
						fill={p.accent}
					/>
					<path
						d="M-26 8Q-20 19 -13 11Q-7 23 0 15Q7 23 13 11Q20 19 26 8M-19 17Q-12 28 -6 20Q0 29 6 20Q12 28 19 17"
						fill="none"
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<circle cx="0" cy="10" r="3" fill={p.sclera} />
				</>
			),
			"accessory",
		),
		outfit(
			"sailor-shirt",
			"Sailor shirt",
			(p) => (
				<>
					<path
						d="M-27 0H-12L0 11L12 0H27L18 14L0 18L-18 14Z"
						fill={p.clothing}
					/>
					<path
						d="M-22 2L-16 10L0 15L16 10L22 2"
						stroke={p.sclera}
						fill="none"
					/>
					<path d="M-3 13H3L6 28L0 24L-6 28Z" fill={p.accent} />
				</>
			),
			"v",
			"sclera",
		),
		outfit("wetsuit", "Wetsuit", (p) => (
			<>
				<path d="M-100 0H-18L-9 32H-100ZM18 0H100V32H9Z" fill={p.accent} />
				<path
					d="M-10 2Q0 8 10 2M0 6V32"
					stroke={p.sclera}
					strokeWidth="2"
					fill="none"
				/>
				<path
					d="M-23 17Q-15 11 -8 17"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2"
				/>
			</>
		)),
	],
	fantasy: [
		outfit("chainmail-hauberk", "Chainmail hauberk", (p) => (
			<>
				<path
					d="M-29 6Q-24 15 -19 6Q-14 15 -9 6M9 6Q14 15 19 6Q24 15 29 6M-33 13Q-28 22 -23 13Q-18 22 -13 13Q-8 22 -3 13Q2 22 7 13Q12 22 17 13Q22 22 27 13Q32 22 37 13M-28 20Q-23 29 -18 20Q-13 29 -8 20Q-3 29 2 20Q7 29 12 20Q17 29 22 20Q27 29 32 20"
					stroke={p.sclera}
					fill="none"
					strokeWidth="1.7"
				/>
				<path
					d="M-14 0Q0 13 14 0M-100 28H100"
					stroke={p.accent}
					fill="none"
					strokeWidth="4"
				/>
				<rect x="-5" y="25" width="10" height="7" rx="1" fill={p.clothing} />
			</>
		)),
		outfit("dragon-scale-cuirass", "Dragon scale cuirass", (p) => (
			<>
				<path
					d="M-32 0L-23 9L-16 2L0 9L16 2L23 9L32 0L29 24L16 31H-16L-29 24Z"
					fill={p.accent}
				/>
				<path
					d="M-25 11Q-18 23 -11 11Q-4 23 3 11Q10 23 17 11Q24 23 31 11M-23 21Q-16 33 -9 21Q-2 33 5 21Q12 33 19 21"
					stroke={p.clothing}
					fill="none"
					strokeWidth="2"
				/>
				<path d="M0 7L6 14L0 21L-6 14Z" fill={p.sclera} />
			</>
		)),
		outfit(
			"ranger-hooded-cloak",
			"Ranger hooded cloak",
			(p) => (
				<>
					<path
						d="M-31 0Q-29 12 -9 8L-20 32H-100V0ZM31 0Q29 12 9 8L20 32H100V0Z"
						fill={p.clothing}
					/>
					<path
						d="M-29 0Q-15 17 0 8Q15 17 29 0M-18 12L-27 30M18 12L27 30"
						stroke={p.accent}
						strokeWidth="2"
						fill="none"
					/>
					<path d="M-6 9Q0 1 6 9Q2 18 -6 9Z" fill={p.accent} />
					<path d="M-3 8L3 11" stroke={p.sclera} />
				</>
			),
			"accessory",
		),
		outfit("knight-armor", "Knight armor", (p) => (
			<>
				<path
					d="M-30 1L-19 0L-15 12L-31 10ZM30 1L19 0L15 12L31 10Z"
					fill={p.accent}
				/>
				<path d="M-16 5Q0 13 16 5L13 24L0 31L-13 24Z" fill={p.sclera} />
				<path
					d="M0 11V27M-11 16L0 21L11 16M-100 26H100"
					fill="none"
					stroke={p.clothing}
					strokeWidth="2"
				/>
			</>
		)),
		outfit(
			"royal-cape",
			"Royal cape",
			(p) => (
				<>
					<path
						d="M-32 0L-8 6L-15 32H-100V0ZM32 0L8 6L15 32H100V0Z"
						fill={p.clothing}
					/>
					<path
						d="M-34 0Q-22 12 -7 8M34 0Q22 12 7 8"
						fill="none"
						stroke={p.sclera}
						strokeWidth="6"
					/>
					<path
						d="M-7 8Q0 18 7 8"
						fill="none"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<circle cx="-8" cy="8" r="3" fill={p.accent} />
					<circle cx="8" cy="8" r="3" fill={p.accent} />
				</>
			),
			"accessory",
		),
		outfit(
			"fairy-petal-collar",
			"Fairy petal collar",
			(p) => (
				<>
					<path
						d="M-28 0Q-31 20 -17 17Q-16 30 -6 22Q0 35 6 22Q16 30 17 17Q31 20 28 0L12 0Q0 9 -12 0Z"
						fill={p.clothing}
					/>
					<path
						d="M-19 6L-17 17M-8 8L-6 22M8 8L6 22M19 6L17 17"
						stroke={p.accent}
					/>
					<circle cx="0" cy="11" r="3" fill={p.accent} />
				</>
			),
			"accessory",
		),
	],
	garden: [
		outfit("florist-wrap-apron", "Florist wrap apron", (p) => (
			<>
				<path d="M-19 0L-24 32H24L19 0L0 10Z" fill={p.accent} />
				<path
					d="M-19 2L14 32M-28 23H28"
					stroke={p.clothing}
					strokeWidth="2"
					fill="none"
				/>
				<path d="M7 18L4 7M12 18L16 5" stroke={p.clothing} strokeWidth="2" />
				<path
					d="M4 4Q-4 0 0 8Q-1 14 6 10Q12 12 10 6Q12 0 4 4ZM16 2Q10 -1 11 5Q9 10 16 8Q23 11 22 5Q24 0 16 2Z"
					fill={p.cheek}
				/>
				<path d="M1 17H21L18 29H4Z" fill={p.sclera} />
			</>
		)),
		outfit(
			"seed-pocket-waistcoat",
			"Seed pocket waistcoat",
			(p) => (
				<>
					<path d="M-10 0L-3 32H3L10 0" fill={p.sclera} />
					<path d="M-28 6H-16V23H-28ZM14 9H27V26H14Z" fill={p.accent} />
					<path
						d="M-28 10H-16M14 13H27M-28 20H-16M14 23H27"
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<path
						d="M-22 17Q-28 11 -20 12Q-17 16 -22 17ZM20 20Q14 14 22 15Q26 18 20 20Z"
						fill={p.clothing}
						stroke="none"
					/>
					<path d="M-13 27H-6M7 28H13" stroke={p.sclera} />
				</>
			),
			"v",
		),
		outfit("strawberry-knit-poncho", "Strawberry knit poncho", (p) => (
			<>
				<path d="M-28 0L-18 11L-9 4L0 13L9 4L18 11L28 0" fill={p.accent} />
				<path
					d="M-23 17Q-17 9 -12 17Q-13 25 -18 28Q-25 23 -23 17ZM10 17Q15 9 22 17Q24 23 16 28Q11 26 10 17Z"
					fill={p.cheek}
				/>
				<path
					d="M-20 18V20M-16 22V24M14 18V20M19 22V24"
					stroke={p.sclera}
					strokeWidth="1.5"
				/>
				<path
					d="M-31 26L-28 32M-6 27V33M6 27V33M31 26L28 32"
					stroke={p.accent}
					strokeWidth="2"
				/>
			</>
		)),
		outfit(
			"gardening-apron",
			"Gardening apron",
			(p) => (
				<>
					<path
						d="M-18 0V10H18V0M-18 10L-27 32H27L18 10Z"
						fill={p.clothing}
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path d="M-15 17H15V27H-15Z" fill={p.accent} />
					<path
						d="M0 24V16M0 20Q-10 19 -6 14Q1 15 0 20M0 18Q8 18 6 12Q0 12 0 18"
						fill={p.clothing}
					/>
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"beekeeper-smock",
			"Beekeeper smock",
			(p) => (
				<>
					<path
						d="M-26 0L-19 9H19L26 0M0 10V32"
						fill="none"
						stroke={p.clothing}
						strokeWidth="2"
					/>
					<path d="M13 12H27V24H13Z" fill={p.accent} />
					<path d="M17 16L20 14L23 16V20L20 22L17 20Z" fill={p.clothing} />
					<path d="M-24 15H-11M-24 20H-11" stroke={p.clothing} />
				</>
			),
			"straight",
			"sclera",
		),
	],
	music: [
		outfit(
			"piano-key-waistcoat",
			"Piano key waistcoat",
			(p) => (
				<>
					<path d="M-31 10H31V29H-31Z" fill={p.sclera} />
					<path
						d="M-23 10V29M-15 10V29M-7 10V29M1 10V29M9 10V29M17 10V29M25 10V29"
						stroke={p.ink}
					/>
					<path
						d="M-26 10H-21V20H-26ZM-18 10H-13V20H-18ZM-2 10H3V20H-2ZM6 10H11V20H6ZM22 10H27V20H22Z"
						fill={p.ink}
					/>
					<path
						d="M-12 0L0 11L12 0"
						fill="none"
						stroke={p.accent}
						strokeWidth="3"
					/>
				</>
			),
			"v",
		),
		outfit(
			"rock-studded-vest",
			"Rock studded vest",
			(p) => (
				<>
					<path
						d="M-14 0L-19 9L-5 15L3 32M14 0L19 9L7 15"
						stroke={p.sclera}
						strokeWidth="2.5"
						fill="none"
					/>
					<path
						d="M-27 3L-24 7L-21 3M21 3L24 7L27 3M-28 12L-25 16L-22 12M22 12L25 16L28 12"
						fill={p.accent}
					/>
					<path
						d="M-25 24L-12 20M14 22L27 26"
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<path d="M-15 19L-12 23" stroke={p.accent} strokeWidth="3" />
				</>
			),
			"v",
		),
		outfit(
			"choir-robe",
			"Choir robe",
			(p) => (
				<>
					<path d="M-30 0H-13L0 9L13 0H30L0 23Z" fill={p.sclera} />
					<path
						d="M-25 1L0 18L25 1"
						stroke={p.accent}
						fill="none"
						strokeWidth="2"
					/>
					<path
						d="M-24 16L-28 32M-14 21L-17 32M0 24V32M14 21L17 32M24 16L28 32"
						stroke={p.accent}
						strokeWidth="1.5"
					/>
					<path d="M-4 9H4L0 14Z" fill={p.clothing} />
				</>
			),
			"v",
		),
		outfit(
			"marching-jacket",
			"Marching jacket",
			(p) => (
				<>
					<path d="M-28 0H-17V6H-28ZM17 0H28V6H17Z" fill={p.accent} />
					<path
						d="M-13 9Q0 14 13 9M-13 15Q0 20 13 15M-13 21Q0 26 13 21M0 1V32"
						fill="none"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<Buttons color={p.sclera} x={-14} />
					<Buttons color={p.sclera} x={14} />
				</>
			),
			"straight",
		),
		outfit(
			"stage-jacket",
			"Stage jacket",
			(p) => (
				<>
					<path d="M-15 0L-20 8L-9 12L0 30L9 12L20 8L15 0" fill={p.accent} />
					<path
						d="M-28 9V18M-24 10V21M-20 11V22M20 11V22M24 10V21M28 9V18"
						stroke={p.sclera}
						strokeWidth="1.5"
					/>
					<path
						d="M0 10L2 16L8 16L3 20L5 26L0 22L-5 26L-3 20L-8 16L-2 16Z"
						fill={p.sclera}
					/>
				</>
			),
			"v",
		),
	],
	adventure: [
		outfit("western-fringe-jacket", "Western fringe jacket", (p) => (
			<>
				<path
					d="M-100 0H-12L0 7L12 0H100V11L24 8L14 15L0 10L-14 15L-24 8L-100 11Z"
					fill={p.accent}
				/>
				<path
					d="M-28 11V20M-23 11V22M-18 14V25M18 14V25M23 11V22M28 11V20"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<path d="M-5 7L-3 21M5 7L3 21" stroke={p.ink} />
				<path d="M0 7L4 11L0 15L-4 11Z" fill={p.sclera} />
				<path
					d="M-13 25L-24 28M13 25L24 28"
					stroke={p.accent}
					strokeWidth="2"
				/>
			</>
		)),
		outfit("cycling-jersey", "Cycling jersey", (p) => (
			<>
				<path d="M-100 0H-20L9 32H-100Z" fill={p.accent} stroke="none" />
				<path d="M-100 16H-9L-4 22H-100Z" fill={p.sclera} stroke="none" />
				<path
					d="M-10 0V6H10V0M0 6V32"
					stroke={p.sclera}
					fill="none"
					strokeWidth="2"
				/>
				<path d="M13 10H27V19H13Z" fill={p.sclera} />
				<path d="M17 13L20 16L24 12" stroke={p.clothing} fill="none" />
				<path d="M-100 29H100" stroke={p.ink} strokeWidth="3" />
			</>
		)),
		outfit(
			"martial-arts-gi",
			"Martial arts gi",
			(p) => (
				<>
					<path
						d="M-14 0L12 28M14 0L-10 27"
						stroke={p.clothing}
						strokeWidth="3"
						fill="none"
					/>
					<path d="M-100 23H100" stroke={p.ink} strokeWidth="5" />
					<path
						d="M-4 20H4V26H-4ZM0 25L-5 33M2 25L9 32"
						stroke={p.ink}
						fill={p.ink}
						strokeWidth="3"
					/>
					<path d="M-30 7L-25 17M30 7L25 17" stroke={p.clothing} opacity=".6" />
				</>
			),
			"v",
			"sclera",
		),
		outfit("hiking-fleece", "Hiking fleece", (p) => (
			<>
				<path
					d="M-12 0V5H12V0M0 6V18"
					fill="none"
					stroke={p.sclera}
					strokeWidth="3"
				/>
				<path d="M-100 16H100V32H-100Z" fill={p.accent} stroke="none" />
				<path
					d="M-24 21L-13 19M13 19L24 21"
					stroke={p.clothing}
					strokeWidth="2"
				/>
				<path d="M17 7L22 13H12Z" fill={p.sclera} />
			</>
		)),
		outfit("expedition-parka", "Expedition parka", (p) => (
			<>
				<path
					d="M-27 0Q-22 11 -6 9M27 0Q22 11 6 9"
					fill="none"
					stroke={p.sclera}
					strokeWidth="7"
				/>
				<path d="M-5 8L-3 27H3L5 8" fill={p.accent} />
				<path
					d="M-28 15H-13V24H-28ZM13 15H28V24H13Z"
					fill="none"
					stroke={p.sclera}
				/>
				<Buttons color={p.clothing} />
			</>
		)),
		outfit(
			"climbing-harness",
			"Climbing harness",
			(p) => (
				<>
					<path
						d="M-23 0L-6 26M23 0L6 26M-100 19H100"
						fill="none"
						stroke={p.clothing}
						strokeWidth="6"
					/>
					<rect x="-7" y="15" width="14" height="9" rx="2" fill={p.accent} />
					<path d="M-3 18H3V21H-3Z" fill={p.ink} />
					<path
						d="M18 18V27Q25 33 26 24V18Z"
						fill="none"
						stroke={p.accent}
						strokeWidth="3"
					/>
				</>
			),
			"accessory",
		),
	],
	arcade: [
		outfit("basketball-jersey", "Basketball jersey", (p) => (
			<>
				<path
					d="M-25 0Q-29 15 -34 18M25 0Q29 15 34 18M-12 0Q0 13 12 0"
					stroke={p.accent}
					strokeWidth="4"
					fill="none"
				/>
				<path
					d="M-12 12H-3V18H-9V23H-3M3 12H12V23H3M5 18H12"
					stroke={p.sclera}
					strokeWidth="3"
					fill="none"
				/>
				<path d="M-100 28H100" stroke={p.accent} strokeWidth="3" />
			</>
		)),
		outfit(
			"football-jersey",
			"Football jersey",
			(p) => (
				<>
					<path
						d="M-100 6H-20M20 6H100M-100 12H-20M20 12H100"
						stroke={p.sclera}
						strokeWidth="3"
					/>
					<path
						d="M-12 0L0 8L12 0"
						stroke={p.accent}
						strokeWidth="3"
						fill="none"
					/>
					<path
						d="M-11 12H-3V25H-11ZM4 12H12V25H4ZM4 18H12"
						stroke={p.sclera}
						strokeWidth="2.5"
						fill="none"
					/>
				</>
			),
			"v",
		),
		outfit("bowling-shirt", "Bowling shirt", (p) => (
			<>
				<path d="M-18 0H-4V32H-18ZM4 0H18V32H4Z" fill={p.sclera} />
				<Collar color={p.accent} />
				<path d="M0 7V32" stroke={p.ink} />
				<Buttons color={p.accent} />
				<path
					d="M21 9Q26 7 25 12L23 17Q29 23 24 26Q18 27 19 22L21 17L20 12Q18 10 21 9Z"
					fill={p.sclera}
				/>
				<path d="M20 14H25" stroke={p.cheek} strokeWidth="2" />
			</>
		)),
		outfit("racing-suit", "Racing suit", (p) => (
			<>
				<path d="M-25 0H-14V32H-25Z" fill={p.sclera} />
				<path
					d="M-25 0H-19V6H-25ZM-19 6H-14V12H-19ZM-25 12H-19V18H-25ZM-19 18H-14V24H-19Z"
					fill={p.ink}
				/>
				<path d="M0 6V32M14 9H26V18H14Z" fill={p.accent} />
				<path d="M17 12H23M17 15H21" stroke={p.clothing} />
			</>
		)),
		outfit(
			"pixel-jersey",
			"Pixel jersey",
			(p) => (
				<>
					<path
						d="M-30 0V7H-22V14H-14M30 0V7H22V14H14"
						fill="none"
						stroke={p.accent}
						strokeWidth="4"
					/>
					<path d="M-8 12H8V17H3V27H-3V17H-8Z" fill={p.sclera} stroke="none" />
					<path
						d="M-11 1L0 8L11 1"
						fill="none"
						stroke={p.sclera}
						strokeWidth="2"
					/>
				</>
			),
			"v",
		),
	],
	pirates: [
		outfit(
			"buccaneer-baldric",
			"Buccaneer baldric",
			(p) => (
				<>
					<path d="M-30 0H-16L32 32H17Z" fill={p.clothing} />
					<path
						d="M-25 1L27 32M-18 1L34 32"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path d="M-10 8L2 16L-4 24L-16 16Z" fill={p.accent} />
					<path d="M-9 12L-3 16L-6 20L-12 16Z" fill={p.clothing} />
					<path d="M15 10H29V25L22 29L15 25Z" fill={p.clothing} />
					<path d="M15 10L22 16L29 10" fill={p.accent} />
					<circle cx="22" cy="17" r="1.5" fill={p.sclera} />
				</>
			),
			"accessory",
		),
		outfit(
			"ruffled-pirate-blouse",
			"Ruffled pirate blouse",
			(p) => (
				<>
					<path d="M-18 0L-11 8L-5 4L0 11L5 4L11 8L18 0" fill={p.sclera} />
					<path
						d="M-5 6Q-14 11 -7 15Q-15 20 -7 23Q-10 29 0 31Q10 29 7 23Q15 20 7 15Q14 11 5 6Z"
						fill={p.sclera}
					/>
					<path
						d="M-6 13H6M-6 21H6M-3 7L3 11M3 7L-3 11"
						stroke={p.clothing}
						fill="none"
					/>
					<path
						d="M-27 8L-21 15M27 8L21 15"
						stroke={p.accent}
						strokeWidth="2"
					/>
				</>
			),
			"v",
			"sclera",
		),
		outfit(
			"naval-greatcoat",
			"Naval greatcoat",
			(p) => (
				<>
					<path d="M-33 0H-18V8H-33ZM18 0H33V8H18Z" fill={p.accent} />
					<path
						d="M-30 8V14M-25 8V14M-20 8V14M20 8V14M25 8V14M30 8V14"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path d="M-13 0L-6 32H6L13 0" fill={p.sclera} />
					<path d="M-15 0L-9 32M15 0L9 32" stroke={p.accent} strokeWidth="2" />
					<Buttons color={p.accent} x={-15} />
					<Buttons color={p.accent} x={15} />
					<path d="M20 18H29V23H20Z" fill={p.cheek} />
				</>
			),
			"straight",
		),
		outfit(
			"pirate-coat",
			"Pirate coat",
			(p) => (
				<>
					<path d="M-11 0H11L6 32H-6Z" fill={p.sclera} />
					<path
						d="M-18 0L-13 14L-8 32M18 0L13 14L8 32"
						stroke={p.accent}
						strokeWidth="3"
						fill="none"
					/>
					<Buttons color={p.accent} x={-20} />
					<Buttons color={p.accent} x={20} />
					<path d="M-7 7L7 11L-7 15L7 19" fill="none" />
				</>
			),
			"straight",
		),
		outfit(
			"deckhand-shirt",
			"Deckhand shirt",
			(p) => (
				<>
					<path
						d="M-100 12H100M-100 24H100"
						stroke={p.clothing}
						strokeWidth="5"
					/>
					<path d="M-24 0L0 17L24 0L8 0L0 7L-8 0Z" fill={p.accent} />
					<path d="M0 13L9 25L0 22L-8 27Z" fill={p.accent} />
				</>
			),
			"round",
			"sclera",
		),
		outfit(
			"treasure-sash",
			"Treasure sash",
			(p) => (
				<>
					<path d="M-30 0H-15L30 32H15Z" fill={p.clothing} />
					<path
						d="M-27 1L24 32M-18 1L33 32"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path d="M-5 9L5 15L0 23L-10 17Z" fill={p.accent} />
					<path d="M-4 13L0 16L-2 19L-6 16Z" fill={p.clothing} />
					<circle cx="17" cy="24" r="4" fill={p.accent} />
				</>
			),
			"accessory",
		),
	],
	arcane: [
		outfit(
			"stargazer-robes",
			"Stargazer robes",
			(p) => (
				<>
					<path
						d="M-18 0L-10 32M18 0L10 32"
						stroke={p.accent}
						strokeWidth="5"
					/>
					<path
						d="M2 12Q-8 12 -6 22Q0 30 8 22Q0 23 2 12Z"
						fill={p.sclera}
						stroke="none"
					/>
					<path
						d="M-28 9V17M-32 13H-24M25 15V23M21 19H29M-23 23V28M-26 25H-20"
						stroke={p.sclera}
						strokeWidth="1.5"
					/>
					<path d="M-6 1L0 7L6 1" stroke={p.sclera} fill="none" />
				</>
			),
			"v",
		),
		outfit(
			"potion-pocket-vest",
			"Potion pocket vest",
			(p) => (
				<>
					<path
						d="M-12 0L0 10L12 0M0 10V32"
						stroke={p.accent}
						fill="none"
						strokeWidth="2"
					/>
					<path
						d="M-26 9H-20V14L-28 21Q-23 29 -18 21L-24 14V9M15 8H21V14L13 22Q18 29 23 22L19 14V8"
						fill={p.sclera}
					/>
					<path d="M-27 20H-19M14 21H22" stroke={p.cheek} strokeWidth="4" />
					<path d="M-30 24H-15V31H-30ZM10 24H27V31H10Z" fill={p.accent} />
					<Buttons color={p.sclera} x={1} />
				</>
			),
			"v",
		),
		outfit(
			"spellwork-cape",
			"Spellwork cape",
			(p) => (
				<>
					<path
						d="M-31 0L-7 8L-15 32H-100V0ZM31 0L7 8L15 32H100V0Z"
						fill={p.clothing}
					/>
					<path
						d="M-8 8Q0 16 8 8"
						stroke={p.accent}
						fill="none"
						strokeWidth="2"
					/>
					<path
						d="M-23 9L-29 21H-17ZM23 9L17 21H29Z"
						stroke={p.accent}
						fill="none"
						strokeWidth="2"
					/>
					<path d="M-26 16H-20M20 16H26" stroke={p.sclera} />
					<circle cx="0" cy="11" r="3" fill={p.cheek} />
					<path
						d="M-31 27H-20M20 27H31"
						stroke={p.accent}
						strokeDasharray="2 3"
						strokeWidth="2"
					/>
				</>
			),
			"accessory",
		),
		outfit(
			"scholar-robes",
			"Scholar robes",
			(p) => (
				<>
					<path d="M-13 0L-4 32H4L13 0L0 10Z" fill={p.sclera} />
					<path d="M-17 0L-9 32M17 0L9 32" stroke={p.accent} strokeWidth="4" />
					<path
						d="M-28 10L-24 6L-20 10L-24 18ZM20 10L24 6L28 10L24 18Z"
						fill={p.accent}
					/>
				</>
			),
			"v",
		),
		outfit("alchemist-apron", "Alchemist apron", (p) => (
			<>
				<path
					d="M-15 0V8H15V0M-15 8L-28 32H28L15 8Z"
					fill={p.clothing}
					stroke={p.accent}
					strokeWidth="2"
				/>
				<path d="M-5 12H5M-3 12V17L-8 24Q0 29 8 24L3 17V12" fill={p.sclera} />
				<path d="M-5 23H5" stroke={p.accent} strokeWidth="3" />
				<path
					d="M-20 18L-16 21M18 14L22 17"
					stroke={p.accent}
					strokeWidth="3"
				/>
			</>
		)),
		outfit(
			"enchanted-collar",
			"Enchanted collar",
			(p) => (
				<>
					<path
						d="M-31 0L-23 17L-12 9L0 28L12 9L23 17L31 0Q0 11 -31 0Z"
						fill={p.clothing}
						stroke={p.accent}
						strokeWidth="1.5"
					/>
					<path d="M0 11L5 17L0 23L-5 17Z" fill={p.accent} />
					<circle cx="-21" cy="8" r="2" fill={p.sclera} />
					<circle cx="21" cy="8" r="2" fill={p.sclera} />
				</>
			),
			"accessory",
		),
	],
	celebrations: [
		outfit(
			"lantern-festival-jacket",
			"Lantern festival jacket",
			(p) => (
				<>
					<path
						d="M-12 0V5H12V0M0 5V32"
						stroke={p.accent}
						strokeWidth="2"
						fill="none"
					/>
					<path
						d="M-6 10Q-10 6 -10 10Q-10 14 -6 10H6Q10 6 10 10Q10 14 6 10M-6 20Q-10 16 -10 20Q-10 24 -6 20H6Q10 16 10 20Q10 24 6 20"
						stroke={p.accent}
						strokeWidth="1.5"
						fill="none"
					/>
					<path
						d="M-24 12L-27 18L-21 18ZM23 12L20 18H26Z"
						fill={p.sclera}
						stroke="none"
					/>
					<path d="M-100 28H100" stroke={p.accent} strokeWidth="3" />
				</>
			),
			"straight",
			"cheek",
		),
		outfit("festival-patchwork-poncho", "Festival patchwork poncho", (p) => (
			<>
				<path d="M-100 7H100V16H-100Z" fill={p.accent} stroke="none" />
				<path d="M-100 24H100" stroke={p.sclera} strokeWidth="4" />
				<path
					d="M-24 8L-18 12L-24 16L-30 12ZM0 8L6 12L0 16L-6 12ZM24 8L30 12L24 16L18 12Z"
					fill={p.clothing}
					stroke="none"
				/>
				<path
					d="M-30 27V33M-20 27V33M-10 27V33M0 27V33M10 27V33M20 27V33M30 27V33"
					stroke={p.accent}
					strokeWidth="3"
				/>
			</>
		)),
		outfit(
			"birthday-ribbon-sash",
			"Birthday ribbon sash",
			(p) => (
				<>
					<path d="M18 0H32L-18 32H-32Z" fill={p.accent} />
					<path
						d="M23 0L-27 32M30 0L-20 32"
						stroke={p.sclera}
						strokeWidth="1.5"
					/>
					<path
						d="M-12 17Q-23 7 -25 16Q-25 23 -12 21Q-2 29 1 21Q4 12 -12 17Z"
						fill={p.cheek}
					/>
					<circle cx="-12" cy="19" r="3" fill={p.sclera} />
					<path
						d="M-13 22L-20 32M-10 22L-6 32"
						stroke={p.cheek}
						strokeWidth="4"
					/>
					<path
						d="M13 7L15 10L19 10L16 13L17 16L13 14L10 16L11 12L8 10L12 10Z"
						fill={p.sclera}
						stroke="none"
					/>
				</>
			),
			"accessory",
		),
		outfit("festive-sweater", "Festive sweater", (p) => (
			<>
				<path d="M-100 12H100M-100 27H100" stroke={p.accent} strokeWidth="3" />
				<path
					d="M0 14V24M-5 19H5M-4 15L4 23M4 15L-4 23M-25 19L-20 14L-15 19L-20 24ZM15 19L20 14L25 19L20 24Z"
					fill="none"
					stroke={p.sclera}
					strokeWidth="1.7"
				/>
			</>
		)),
		outfit(
			"party-waistcoat",
			"Party waistcoat",
			(p) => (
				<>
					<path
						d="M-11 0L0 10L11 0M0 10V32"
						stroke={p.sclera}
						strokeWidth="2"
						fill="none"
					/>
					<Buttons color={p.sclera} />
					<path
						d="M-26 5L-22 10M-16 18L-20 23M19 5L16 10M22 19L27 17M-29 20L-26 22"
						stroke={p.accent}
						strokeWidth="3"
					/>
					<path d="M12 14H25L20 10Z" fill={p.sclera} />
				</>
			),
			"v",
		),
		outfit(
			"pumpkin-poncho",
			"Pumpkin poncho",
			(p) => (
				<>
					<path
						d="M-25 0L-17 10L-8 5L0 13L8 5L17 10L25 0Q0 7 -25 0Z"
						fill={p.clothing}
					/>
					<path
						d="M-20 14L-12 21H-24ZM20 14L24 21H12ZM-9 24L-4 27L0 24L4 27L9 24"
						fill={p.clothing}
					/>
					<path
						d="M-32 6Q-36 18 -31 30M32 6Q36 18 31 30"
						fill="none"
						stroke={p.clothing}
						opacity=".5"
					/>
				</>
			),
			"round",
			"accent",
		),
	],
	service: [
		outfit(
			"rail-conductor-coat",
			"Rail conductor coat",
			(p) => (
				<>
					<Collar color={p.accent} />
					<path
						d="M-13 0L-7 30H7L13 0"
						fill={p.clothing}
						stroke={p.accent}
						strokeWidth="2"
					/>
					<Buttons color={p.sclera} x={-3} />
					<Buttons color={p.sclera} x={5} />
					<path d="M16 10H28V18H16Z" fill={p.sclera} />
					<path d="M19 14H25" stroke={p.clothing} />
					<path
						d="M-24 15Q-18 32 -8 22"
						stroke={p.accent}
						fill="none"
						strokeWidth="2"
					/>
					<circle cx="-24" cy="15" r="3" fill={p.accent} />
				</>
			),
			"straight",
		),
		outfit("postal-uniform", "Postal uniform", (p) => (
			<>
				<Collar color={p.sclera} />
				<path d="M-28 0H-19L26 32H17Z" fill={p.accent} />
				<path d="M0 7V32" stroke={p.sclera} />
				<Buttons color={p.sclera} />
				<path
					d="M14 8H29V18H14ZM14 8L21 14L29 8"
					fill={p.sclera}
					stroke={p.clothing}
				/>
				<path d="M-28 18H-14V28H-28Z" fill={p.accent} />
				<path d="M-28 18L-21 23L-14 18" fill="none" />
			</>
		)),
		outfit(
			"coastguard-flotation-jacket",
			"Coastguard flotation jacket",
			(p) => (
				<>
					<path
						d="M-30 0H-14L-5 10V31H-31ZM30 0H14L5 10V31H31Z"
						fill={p.accent}
					/>
					<path
						d="M-28 9H-13M13 9H28M-28 15H-13M13 15H28"
						stroke={p.sclera}
						strokeWidth="3"
					/>
					<path d="M-100 23H100" stroke={p.clothing} strokeWidth="5" />
					<rect x="-6" y="19" width="12" height="8" rx="2" fill={p.sclera} />
					<path d="M-2 22H2V25H-2Z" fill={p.clothing} />
				</>
			),
			"v",
		),
		outfit(
			"courier-vest",
			"Courier vest",
			(p) => (
				<>
					<path
						d="M-26 0L-18 28M26 0L18 28M-100 22H100"
						stroke={p.sclera}
						strokeWidth="4"
						fill="none"
					/>
					<path d="M0 6V32" stroke={p.clothing} strokeWidth="2" />
					<path d="M9 10H23V18H9Z" fill={p.clothing} />
					<path d="M12 14H20M17 11L20 14L17 17" fill="none" stroke={p.sclera} />
				</>
			),
			"v",
			"accent",
		),
		outfit(
			"pilot-uniform",
			"Pilot uniform",
			(p) => (
				<>
					<Collar color={p.sclera} />
					<path d="M-3 6H3L4 23L0 27L-4 23Z" fill={p.clothing} />
					<path d="M-30 2H-18V7H-30ZM18 2H30V7H18Z" fill={p.clothing} />
					<path
						d="M-27 3V6M-23 3V6M21 3V6M25 3V6"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<path d="M12 13H27L20 17Z" fill={p.accent} />
				</>
			),
			"straight",
			"sclera",
		),
	],
	workplace: [
		outfit(
			"barbershop-waistcoat",
			"Barbershop waistcoat",
			(p) => (
				<>
					<path
						d="M-27 0V32M-20 0V32M-13 4V32M13 4V32M20 0V32M27 0V32"
						stroke={p.sclera}
						opacity=".5"
					/>
					<path
						d="M-12 0L0 11L12 0M0 11V32"
						stroke={p.accent}
						fill="none"
						strokeWidth="2"
					/>
					<path d="M15 8H24V19H15Z" fill={p.sclera} />
					<path d="M17 10V16M20 10V16M23 10V16" stroke={p.clothing} />
					<path d="M12 17H28V27H12Z" fill={p.accent} />
					<Buttons color={p.accent} />
				</>
			),
			"v",
		),
		outfit(
			"museum-guide-jacket",
			"Museum guide jacket",
			(p) => (
				<>
					<path
						d="M-16 0L-21 8L-8 14L0 29L8 14L21 8L16 0"
						fill={p.clothing}
						stroke={p.sclera}
						strokeWidth="2"
					/>
					<path
						d="M-21 7Q-13 11 -8 7"
						stroke={p.accent}
						strokeWidth="3"
						fill="none"
					/>
					<circle cx="-15" cy="9" r="2" fill={p.accent} />
					<rect x="15" y="13" width="12" height="12" rx="1" fill={p.sclera} />
					<path
						d="M17 17L21 14L25 17M18 18V22M21 18V22M24 18V22M17 23H25"
						stroke={p.clothing}
						fill="none"
					/>
					<circle cx="0" cy="27" r="2" fill={p.accent} />
				</>
			),
			"v",
		),
		outfit(
			"camera-crew-vest",
			"Camera crew vest",
			(p) => (
				<>
					<path d="M-10 0L-4 32H4L10 0" fill={p.sclera} />
					<path
						d="M-27 5H-14V15H-27ZM14 5H27V15H14ZM-29 20H-13V31H-29ZM13 20H29V31H13Z"
						fill={p.accent}
					/>
					<path
						d="M-24 9H-17M17 9H24M-25 24V28M-20 24V28M17 24H25"
						stroke={p.clothing}
						strokeWidth="2"
					/>
					<path
						d="M-16 0Q0 25 16 0"
						stroke={p.ink}
						fill="none"
						strokeWidth="2"
					/>
					<rect x="-6" y="14" width="12" height="9" rx="2" fill={p.clothing} />
					<circle cx="0" cy="18.5" r="3" fill={p.sclera} />
				</>
			),
			"v",
		),
		outfit(
			"trench-coat",
			"Trench coat",
			(p) => (
				<>
					<path d="M-16 0L-23 7L-10 12L0 25L10 12L23 7L16 0" fill={p.accent} />
					<Buttons color={p.ink} x={-7} />
					<Buttons color={p.ink} x={7} />
					<path d="M-100 25H100" stroke={p.accent} strokeWidth="5" />
					<rect x="-5" y="22" width="10" height="7" rx="1" fill="none" />
				</>
			),
			"v",
		),
		outfit(
			"presentation-shirt",
			"Presentation shirt",
			(p) => (
				<>
					<Collar color={p.sclera} />
					<path
						d="M-24 0L-17 32M24 0L17 32"
						stroke={p.clothing}
						strokeWidth="4"
					/>
					<path d="M0 7V32" stroke={p.clothing} />
					<Buttons color={p.clothing} />
					<path
						d="M-18 7L0 19L18 7"
						fill="none"
						stroke={p.accent}
						strokeWidth="2"
					/>
					<rect x="-5" y="17" width="10" height="10" rx="1" fill={p.sclera} />
					<path d="M-2 20H2M-2 23H2" stroke={p.clothing} />
				</>
			),
			"round",
			"sclera",
		),
	],
	healthcare: [
		outfit("therapy-polo", "Therapy polo", (p) => (
			<>
				<Collar color={p.sclera} />
				<path
					d="M-28 0L-16 11M28 0L16 11M-100 25H100"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path d="M-2 7V16H2V7" fill={p.sclera} />
				<path
					d="M14 18Q20 8 26 18M14 18Q20 28 26 18"
					stroke={p.sclera}
					strokeWidth="2"
					fill="none"
				/>
				<circle cx="20" cy="18" r="2" fill={p.accent} />
			</>
		)),
		outfit(
			"pharmacy-wrap-coat",
			"Pharmacy wrap coat",
			(p) => (
				<>
					<path
						d="M-14 0L11 20V32M14 0L2 11"
						stroke={p.clothing}
						fill="none"
						strokeWidth="2"
					/>
					<Buttons color={p.accent} x={12} />
					<path d="M-27 12H-12V24H-27Z" fill={p.accent} />
					<path
						d="M-24 16H-15Q-15 21 -19 21Q-24 21 -24 16ZM-18 16L-13 10"
						fill={p.sclera}
						stroke={p.clothing}
					/>
					<path d="M-31 28H-14M21 27H32" stroke={p.clothing} />
				</>
			),
			"v",
			"sclera",
		),
		outfit(
			"lab-barrier-gown",
			"Lab barrier gown",
			(p) => (
				<>
					<path
						d="M-16 0Q0 12 16 0M-30 3L-24 28M30 3L24 28M0 9V32"
						stroke={p.sclera}
						strokeWidth="2"
						fill="none"
					/>
					<path d="M-100 24H100" stroke={p.sclera} strokeWidth="4" />
					<path
						d="M14 21Q5 14 7 23L14 25Q23 16 25 23L14 25L10 32M16 25L23 32"
						fill={p.sclera}
					/>
					<path d="M-14 14L-18 18L-14 22L-10 18Z" fill={p.accent} />
					<path d="M-14 16V20M-16 18H-12" stroke={p.clothing} />
				</>
			),
			"round",
			"cheek",
		),
		outfit(
			"veterinary-tunic",
			"Veterinary tunic",
			(p) => (
				<>
					<path
						d="M-12 0L0 10L12 0"
						fill="none"
						stroke={p.accent}
						strokeWidth="3"
					/>
					<path d="M13 10H27V23H13Z" fill={p.sclera} />
					<g fill={p.clothing} stroke="none">
						<ellipse cx="20" cy="19" rx="3.5" ry="2.5" />
						<circle cx="16" cy="15" r="1.3" />
						<circle cx="20" cy="14" r="1.3" />
						<circle cx="24" cy="15" r="1.3" />
					</g>
					<path d="M-26 13H-14" stroke={p.sclera} strokeWidth="2" />
				</>
			),
			"v",
		),
		outfit(
			"dental-tabard",
			"Dental tabard",
			(p) => (
				<>
					<path
						d="M-26 0L-20 29M26 0L20 29M-12 1Q0 12 12 1"
						stroke={p.clothing}
						strokeWidth="2"
						fill="none"
					/>
					<path
						d="M-8 12Q-8 8 -3 10Q0 12 3 10Q8 8 8 12Q8 23 4 25L1 19H-1L-4 25Q-8 23 -8 12Z"
						fill={p.sclera}
					/>
					<path d="M-27 17H-18M18 17H27" stroke={p.clothing} strokeWidth="3" />
				</>
			),
			"round",
			"accent",
		),
	],
};
