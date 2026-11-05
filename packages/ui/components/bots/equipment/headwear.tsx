import type { ReactNode } from "react";
import { getBotHeadwearFit } from "../headwear-fit";
import type {
	BotEquipmentDefinition,
	BotPalette,
	BotRenderContext,
} from "../types";

// Kero's raised eyes intersect these brims and side pieces.
const KERO_INCOMPATIBLE_HEADWEAR = new Set([
	"bucket-hat",
	"butterfly-hairclip",
	"puppy-ear-band",
	"hamster-hood",
	"elephant-ear-band",
	"soft-serve-hat",
	"mission-cap",
	"paper-boat-hat",
	"knight-helmet",
	"winged-helmet",
	"straw-sunhat",
	"flower-crown",
	"aviator-cap",
	"tennis-visor",
	"ski-earflap-hat",
	"chess-rook-hat",
	"pirate-headscarf",
	"press-visor",
]);

function headwear(
	id: string,
	label: string,
	draw: (
		palette: BotPalette,
		fit: ReturnType<typeof getBotHeadwearFit>,
	) => ReactNode,
): BotEquipmentDefinition {
	return {
		id,
		label,
		slot: "head",
		...(KERO_INCOMPATIBLE_HEADWEAR.has(id) ? { excludedKinds: ["kero"] } : {}),
		render: ({ anchors, palette }) => {
			const fit = getBotHeadwearFit(anchors);
			return (
				<g
					transform={fit.transform}
					fill={palette.clothing}
					stroke={palette.ink}
					strokeWidth="1.4"
					strokeLinejoin="round"
					strokeLinecap="round"
				>
					{draw(palette, fit)}
				</g>
			);
		},
	};
}

function eyewear(
	id: string,
	label: string,
	draw: (context: BotRenderContext) => ReactNode,
): BotEquipmentDefinition {
	return {
		id,
		label,
		slot: "eyes",
		render: (context) => (
			<g
				fill="none"
				stroke={context.palette.ink}
				strokeWidth="1.5"
				strokeLinejoin="round"
				strokeLinecap="round"
			>
				{draw(context)}
			</g>
		),
	};
}

function LensPair({
	context: { anchors, palette },
	children,
	bridge = true,
}: { context: BotRenderContext; children: ReactNode; bridge?: boolean }) {
	const { left, right, radius } = anchors.eyes;
	const r = radius + 2.5;
	return (
		<>
			{bridge && (
				<path
					d={`M${left[0] + r} ${left[1]}Q${(left[0] + right[0]) / 2} ${(left[1] + right[1]) / 2 - 3} ${right[0] - r} ${right[1]}M${left[0] - r} ${left[1] - 1}h-3M${right[0] + r} ${right[1] - 1}h3`}
					stroke={palette.clothing}
				/>
			)}
			{(["left", "right"] as const).map((side) => (
				<g
					key={side}
					transform={`translate(${anchors.eyes[side][0]} ${anchors.eyes[side][1]}) scale(${r / 7})`}
				>
					{children}
				</g>
			))}
		</>
	);
}

const everydayHeadwearBySeries: Readonly<
	Record<string, readonly BotEquipmentDefinition[]>
> = {
	flow: [
		headwear("flow-snapback", "Flow snapback", (p) => (
			<>
				<path d="M-22 1V-5C-22 -24 21 -24 21 -5V1Z" />
				<path d="M-23 -1H24L30 5H-3Z" fill={p.accent} />
				<path
					d="M-7 -14H3V-10H-2V-6H-7ZM5 -14H10V-6H5Z"
					fill={p.sclera}
					stroke="none"
				/>
			</>
		)),
	],
	basics: [
		headwear("baseball-cap", "Baseball cap", (p) => (
			<>
				<path d="M-22 1V-4C-22 -24 22 -24 22 -4V1Z" />
				<path d="M-2 -18Q6 -13 6 -2M-22 -1H23Q32 4 29 6H2Z" fill={p.accent} />
				<circle cx="0" cy="-20" r="2" fill={p.accent} />
			</>
		)),
		headwear("bucket-hat", "Bucket hat", (p) => (
			<>
				<path d="M-17 -18H17L21 -2H-21Z" />
				<path d="M-21 -4L-29 5Q0 12 29 5L21 -4Z" fill={p.accent} />
				<path d="M-17 -8H17M-24 3Q0 9 24 3" fill="none" />
				<circle cx="10" cy="-13" r="1.2" fill={p.sclera} stroke="none" />
			</>
		)),
		headwear("ribbed-beanie", "Ribbed beanie", (p) => (
			<>
				<path d="M-22 2V-7C-22 -27 22 -27 22 -7V2Z" />
				<path
					d="M-12 -17L-14 -4M-4 -20L-5 -4M5 -20L6 -4M13 -16L15 -4"
					fill="none"
					stroke={p.accent}
				/>
				<rect x="-23" y="-4" width="46" height="9" rx="3" fill={p.accent} />
				<rect x="8" y="-2" width="8" height="5" rx="1" fill={p.sclera} />
			</>
		)),
		eyewear("square-frames", "Square frames", (c) => (
			<LensPair context={c}>
				<rect x="-7" y="-6" width="14" height="12" rx="2" strokeWidth="2.2" />
			</LensPair>
		)),
	],
	nature: [
		headwear("leaf-wreath", "Leaf wreath", (p) => (
			<>
				<path d="M-23 2Q0 -10 23 2" fill="none" strokeWidth="3" />
				<path
					d="M-20 0Q-30 -10 -17 -13Q-11 -6 -20 0ZM-10 -4Q-17 -19 -4 -19Q2 -10 -10 -4ZM2 -5Q-2 -20 11 -18Q15 -7 2 -5ZM14 -2Q16 -17 28 -10Q28 1 14 -2Z"
					fill={p.accent}
				/>
			</>
		)),
		headwear("mushroom-hat", "Mushroom hat", (p) => (
			<>
				<path
					d="M-28 1Q-23 -22 0 -22Q23 -22 28 1Q0 10 -28 1Z"
					fill={p.accent}
				/>
				<path d="M-26 1Q0 -6 26 1" fill="none" />
				<g fill={p.sclera} stroke="none">
					<ellipse cx="-14" cy="-9" rx="4" ry="3" />
					<ellipse cx="5" cy="-15" rx="5" ry="3" />
					<circle cx="17" cy="-6" r="3" />
				</g>
			</>
		)),
		headwear("acorn-hat", "Acorn cap", (p) => (
			<>
				<path d="M-23 0Q-21 -17 0 -17Q21 -17 23 0Z" />
				<path d="M-3 -17Q-5 -24 3 -24L5 -20L2 -16" fill={p.accent} />
				<path
					d="M-20 -1L-8 -13L6 0L17 -10M-16 -9L-5 1L7 -12L20 -1"
					fill="none"
					stroke={p.accent}
				/>
				<path d="M-24 0H24" strokeWidth="4" />
			</>
		)),
	],
	pets: [
		headwear("cat-ear-band", "Cat ear headband", (p) => (
			<>
				<path d="M-23 6Q-24 -7 0 -8Q24 -7 23 6" fill="none" strokeWidth="4" />
				<path d="M-21 -2L-23 -22L-7 -10ZM7 -10L23 -22L21 -2Z" />
				<path
					d="M-18 -7L-19 -15L-12 -10ZM12 -10L19 -15L18 -7Z"
					fill={p.cheek}
					stroke="none"
				/>
			</>
		)),
		headwear("puppy-ear-band", "Puppy ear headband", (p) => (
			<>
				<path d="M-24 2Q0 -17 24 2" fill="none" strokeWidth="4" />
				<path d="M-19 -7Q-29 -10 -30 5Q-32 21 -24 20Q-16 18 -15 0ZM19 -7Q29 -10 30 5Q32 21 24 20Q16 18 15 0Z" />
				<path
					d="M-24 0Q-28 11 -24 14M24 0Q28 11 24 14"
					fill="none"
					stroke={p.cheek}
					strokeWidth="3"
				/>
			</>
		)),
	],
	zoo: [
		headwear("antler-band", "Antler headband", (p) => (
			<>
				<path d="M-22 3Q0 -8 22 3" fill="none" strokeWidth="4" />
				<path
					d="M-15 -1L-19 -12L-17 -21M-19 -12L-25 -15V-18M15 -1L19 -12L17 -21M19 -12L25 -15V-18"
					fill="none"
					stroke={p.accent}
					strokeWidth="4"
				/>
				<path d="M-15 0Q-24 -6 -28 1Q-22 7 -15 0ZM15 0Q24 -6 28 1Q22 7 15 0Z" />
			</>
		)),
		headwear("panda-ear-band", "Panda ear headband", (p) => (
			<>
				<path
					d="M-23 5Q-18 -12 0 -12Q18 -12 23 5"
					fill="none"
					stroke={p.sclera}
					strokeWidth="5"
				/>
				<circle cx="-19" cy="-12" r="9" />
				<circle cx="19" cy="-12" r="9" />
				<circle cx="-19" cy="-12" r="4" fill={p.cheek} stroke="none" />
				<circle cx="19" cy="-12" r="4" fill={p.cheek} stroke="none" />
			</>
		)),
	],
	tech: [
		headwear("antenna-band", "Antenna headband", (p) => (
			<>
				<path d="M-23 4Q0 -7 23 4" fill="none" strokeWidth="4" />
				<path d="M-12 -2L-16 -18M12 -2L16 -18" fill="none" strokeWidth="2" />
				<circle cx="-16" cy="-20" r="4" fill={p.accent} />
				<circle cx="16" cy="-20" r="4" fill={p.accent} />
				<path d="M-4 -5H4V1H-4Z" fill={p.sclera} />
			</>
		)),
		headwear("satellite-hat", "Satellite hat", (p) => (
			<>
				<path d="M-22 2Q-21 -16 0 -16Q21 -16 22 2Z" />
				<path d="M-18 -2H18" stroke={p.accent} strokeWidth="3" />
				<path d="M7 -16L12 -23M4 -23Q16 -13 22 -23Z" fill={p.sclera} />
				<circle cx="13" cy="-23" r="1.5" fill={p.accent} />
			</>
		)),
		eyewear("cyber-visor", "Cyber visor", (c) => {
			const { left, right, radius } = c.anchors.eyes;
			const y = (left[1] + right[1]) / 2;
			return (
				<>
					<path
						d={`M${left[0] - radius - 5} ${y - radius - 3}H${right[0] + radius + 5}L${right[0] + radius + 2} ${y + radius + 3}H${left[0] - radius - 2}Z`}
						fill={c.palette.clothing}
					/>
					<path
						d={`M${left[0] - radius} ${y}H${right[0] + radius}`}
						stroke={c.palette.accent}
						strokeWidth="2.5"
					/>
					<path
						d={`M${right[0]} ${y - radius - 1}l-4 ${radius * 2 + 2}`}
						stroke={c.palette.sclera}
						opacity=".6"
					/>
				</>
			);
		}),
	],
	snacks: [
		headwear("chef-toque", "Chef's toque", (p) => (
			<>
				<path
					d="M-18 -7C-32 -8 -29 -23 -17 -21C-10 -29 -1 -26 2 -21C13 -29 30 -21 24 -12Q23 -7 17 -7V4H-18Z"
					fill={p.sclera}
				/>
				<path d="M-18 -3H17M-9 -13V-5M1 -15V-5M10 -12V-5" fill="none" />
			</>
		)),
		headwear("baker-cap", "Baker cap", (p) => (
			<>
				<path
					d="M-21 -4Q-29 -17 -12 -20Q0 -29 15 -20Q30 -18 22 -4Z"
					fill={p.sclera}
				/>
				<path d="M-21 -4H22V4H-21Z" />
				<path d="M-11 -16L-7 -6M3 -19L3 -6M15 -15L11 -6" fill="none" />
			</>
		)),
		headwear("waffle-bow", "Waffle bow", (p) => (
			<>
				<path
					d="M-3 -6L-21 -18L-23 0L-3 -3ZM3 -6L21 -18L23 0L3 -3Z"
					fill={p.accent}
				/>
				<path
					d="M-18 -13L-10 -7M-20 -8L-15 -4M18 -13L10 -7M20 -8L15 -4M-15 -13L-19 -4M15 -13L19 -4"
					fill="none"
				/>
				<rect x="-4" y="-9" width="8" height="9" rx="2" fill={p.cheek} />
			</>
		)),
	],
	cosmos: [
		headwear("mission-cap", "Mission cap", (p) => (
			<>
				<path d="M-22 4V-7Q-22 -22 0 -22Q22 -22 22 -7V4L14 7V-3H-14V7Z" />
				<path d="M-8 -20V-4H8V-20" fill={p.sclera} />
				<path d="M-3 -12L0 -17L3 -12L0 -7Z" fill={p.accent} />
			</>
		)),
		headwear("orbit-headband", "Orbit headband", (p) => (
			<>
				<path d="M-23 3Q0 -6 23 3M0 -3V-14" fill="none" strokeWidth="3" />
				<circle cx="0" cy="-17" r="7" fill={p.accent} />
				<ellipse
					cx="0"
					cy="-17"
					rx="14"
					ry="3"
					transform="rotate(-20 0 -17)"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2.5"
				/>
				<circle cx="19" cy="-9" r="2" fill={p.sclera} stroke="none" />
			</>
		)),
	],
	pro: [
		headwear("fedora", "Fedora", (p) => (
			<>
				<path d="M-19 -3L-15 -21L0 -17L15 -21L19 -3Z" />
				<path d="M-18 -9L18 -9L19 -3H-19Z" fill={p.accent} />
				<path d="M-29 -1Q0 -8 29 -1L24 5Q0 1 -24 5Z" />
			</>
		)),
		headwear("top-hat", "Top hat", (p) => (
			<>
				<path d="M-17 -23H17L14 -1H-14Z" />
				<path d="M-15 -9H15L14 -2H-14Z" fill={p.accent} />
				<ellipse cx="0" cy="1" rx="28" ry="5" />
				<path d="M-11 -19L-10 -13" stroke={p.sclera} opacity=".55" />
			</>
		)),
		eyewear("monocle", "Monocle", ({ anchors: { eyes }, palette }) => (
			<>
				<circle
					cx={eyes.right[0]}
					cy={eyes.right[1]}
					r={eyes.radius + 3}
					stroke={palette.accent}
					strokeWidth="2"
				/>
				<path
					d={`M${eyes.right[0] + eyes.radius + 3} ${eyes.right[1]}q6 19 -2 25`}
					stroke={palette.accent}
				/>
				<path
					d={`M${eyes.right[0] - 2} ${eyes.right[1] - 3}l3 -1`}
					stroke={palette.sclera}
				/>
			</>
		)),
		eyewear("half-moon-frames", "Half-moon frames", (c) => (
			<LensPair context={c}>
				<path d="M-7 0H7Q7 7 0 7Q-7 7 -7 0Z" strokeWidth="2" />
			</LensPair>
		)),
	],
	office: [
		headwear("pencil-hairpin", "Pencil hairpin", (p) => (
			<>
				<path
					d="M-18 3Q-13 -10 0 -10Q13 -10 18 3"
					fill="none"
					strokeWidth="4"
				/>
				<path d="M-22 -10L18 -20L20 -14L-20 -4L-28 -5Z" fill={p.accent} />
				<path d="M14 -19L16 -13L20 -14L18 -20Z" fill={p.cheek} />
				<path d="M-22 -10L-20 -4L-28 -5Z" fill={p.sclera} />
				<path d="M-28 -5L-24 -5" strokeWidth="2" />
			</>
		)),
		headwear("thinking-cap", "Thinking cap", (p) => (
			<>
				<path d="M-22 1Q-21 -13 0 -13Q21 -13 22 1Z" />
				<path
					d="M-4 -13V-17Q-9 -21 -5 -24Q0 -28 5 -24Q9 -21 4 -17V-13Z"
					fill={p.accent}
				/>
				<path d="M-4 -15H4M-13 -21H-10M10 -21H13" fill="none" />
			</>
		)),
	],
	workshop: [
		headwear("workshop-bandana", "Workshop bandana", (p) => (
			<>
				<path d="M-22 1Q-21 -18 0 -18Q21 -18 22 1Z" fill={p.accent} />
				<path d="M-22 -3Q0 -9 22 -3V3Q0 -2 -22 3Z" />
				<path d="M19 1L28 -6L30 3L23 6L28 16L20 13L17 3Z" fill={p.accent} />
				<path d="M-11 -12L-8 -9M1 -14L4 -11M11 -11L14 -8" stroke={p.sclera} />
			</>
		)),
		eyewear("safety-goggles", "Safety goggles", (c) => (
			<LensPair context={c}>
				<rect
					x="-8"
					y="-6.5"
					width="16"
					height="13"
					rx="4"
					stroke={c.palette.accent}
					strokeWidth="3.5"
				/>
				<path d="M-5 -3L-2 -4M3 3L5 1" stroke={c.palette.sclera} />
			</LensPair>
		)),
	],
	ocean: [
		headwear("sailor-hat", "Sailor hat", (p) => (
			<>
				<path d="M-20 -7Q0 -21 20 -7L17 3H-17Z" fill={p.sclera} />
				<path d="M-23 -9Q0 1 23 -9L20 4Q0 10 -20 4Z" fill={p.sclera} />
				<path d="M-2 -3V4M-6 1Q-2 6 2 1M-4 -2H0" fill="none" />
			</>
		)),
		headwear("coral-crown", "Coral crown", (p) => (
			<>
				<path
					d="M-21 3Q0 -4 21 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="5"
				/>
				<path
					d="M-16 1L-20 -13M-20 -10L-27 -15M-20 -13L-18 -20M-5 -1V-16M-5 -11L-11 -17M8 -1L10 -20M10 -13L17 -18M19 2L24 -11"
					fill="none"
					stroke={p.cheek}
					strokeWidth="3.5"
				/>
				<circle cx="1" cy="0" r="3" fill={p.sclera} />
			</>
		)),
		eyewear("swim-goggles", "Swim goggles", (c) => (
			<LensPair context={c}>
				<ellipse
					cx="0"
					cy="0"
					rx="7.5"
					ry="5.5"
					stroke={c.palette.accent}
					strokeWidth="3"
				/>
				<path d="M-4 -2L0 -3" stroke={c.palette.sclera} strokeWidth="2" />
			</LensPair>
		)),
	],
	fantasy: [
		headwear("dragon-horns", "Dragon horns", (p) => (
			<>
				<path d="M-23 4Q0 -7 23 4" fill="none" strokeWidth="4" />
				<path
					d="M-20 0Q-30 -10 -21 -23Q-22 -10 -10 -5ZM10 -5Q22 -10 21 -23Q30 -10 20 0Z"
					fill={p.accent}
				/>
				<path d="M-3 -3L0 -12L4 -3Z" fill={p.cheek} />
			</>
		)),
		headwear("jester-hat", "Jester hat", (p) => (
			<>
				<path
					d="M-22 2Q-29 -11 -27 -17Q-15 -19 -9 -6Q-3 -23 5 -23Q12 -20 12 -7Q22 -18 28 -13L22 2Z"
					fill={p.accent}
				/>
				<path d="M-9 -6L-7 2H8L12 -7Q12 -20 5 -23Q-3 -23 -9 -6Z" />
				<path d="M-22 0H22V5H-22Z" fill={p.cheek} />
				<g fill={p.sclera}>
					<circle cx="-27" cy="-17" r="3" />
					<circle cx="5" cy="-23" r="3" />
					<circle cx="28" cy="-13" r="3" />
				</g>
			</>
		)),
		headwear("knight-helmet", "Knight's helmet", (p) => (
			<>
				<path
					d="M-22 5V-4Q-22 -23 0 -23Q22 -23 22 -4V5L14 8V-2H-14V8Z"
					fill={p.sclera}
				/>
				<path d="M-3 -23V-3H3V-23" fill={p.accent} />
				<path d="M-21 -3H21V2H-21Z" />
				<path d="M-15 -9H-7M7 -9H15" fill="none" />
			</>
		)),
		eyewear("masquerade-mask", "Masquerade mask", (c) => (
			<LensPair context={c}>
				<path
					d="M-10 -7L-7 5Q0 11 7 4L10 -7L0 -4ZM-5 -2Q0 -5 5 -2L4 3Q0 5 -4 3Z"
					fill={c.palette.accent}
					fillRule="evenodd"
				/>
				<path d="M-7 -6L-5 -10M7 -6L5 -10" stroke={c.palette.clothing} />
			</LensPair>
		)),
	],
	garden: [
		headwear("straw-sunhat", "Straw sunhat", (p) => (
			<>
				<ellipse cx="0" cy="1" rx="30" ry="7" fill={p.accent} />
				<path d="M-18 0L-15 -18Q0 -25 15 -18L18 0Z" fill={p.accent} />
				<path d="M-17 -7H17V-1H-17Z" />
				<path
					d="M-25 3Q0 9 25 3M-10 -17L-12 -9M0 -19V-9M10 -17L12 -9"
					fill="none"
					opacity=".5"
				/>
			</>
		)),
		headwear("flower-crown", "Flower crown", (p) => (
			<>
				<path d="M-24 3Q0 -11 24 3" fill="none" strokeWidth="3" />
				{[-17, 0, 17].map((x) => (
					<g key={x} transform={`translate(${x} ${x === 0 ? -8 : -3})`}>
						<path
							d="M0 -3C-8 -11 -11 -1 -5 2C-10 10 1 13 3 6C12 8 14 -3 6 -4C8 -12 -2 -13 0 -3Z"
							fill={p.cheek}
						/>
						<circle cx="1" cy="0" r="3" fill={p.accent} />
					</g>
				))}
			</>
		)),
	],
	music: [
		headwear("conductor-cap", "Conductor cap", (p) => (
			<>
				<path d="M-22 -4L-27 -13Q0 -27 27 -13L22 -4Z" />
				<path d="M-22 -5H22V2H-22Z" fill={p.accent} />
				<path d="M-17 2Q0 14 17 2Z" />
				<path
					d="M-5 -9V-16L5 -18V-11M-5 -16L5 -18"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2"
				/>
				<ellipse cx="-7" cy="-9" rx="3" ry="2" fill={p.sclera} stroke="none" />
				<ellipse cx="3" cy="-11" rx="3" ry="2" fill={p.sclera} stroke="none" />
			</>
		)),
		headwear("jazz-beret", "Jazz beret", (p) => (
			<>
				<path d="M-23 -2Q-30 -16 -12 -19Q13 -26 24 -14Q28 -3 14 1Z" />
				<path d="M-18 -1H18V4H-18Z" fill={p.accent} />
				<path d="M2 -21L5 -25" fill="none" strokeWidth="3" />
				<path
					d="M-15 -12Q-8 -18 0 -17"
					fill="none"
					stroke={p.sclera}
					opacity=".5"
				/>
			</>
		)),
		eyewear("star-glasses", "Star glasses", (c) => (
			<LensPair context={c}>
				<path
					d="M0 -9L3 -3L9 -3L5 2L6 9L0 5L-6 9L-5 2L-9 -3L-3 -3Z"
					stroke={c.palette.accent}
					strokeWidth="2.5"
				/>
			</LensPair>
		)),
	],
	adventure: [
		headwear("explorer-hat", "Explorer hat", (p) => (
			<>
				<path d="M-19 -3L-16 -17Q0 -26 16 -17L19 -3Z" fill={p.accent} />
				<path d="M-18 -8H18V-2H-18Z" />
				<path d="M-29 -1Q0 -5 29 -1L25 6H-25Z" fill={p.accent} />
				<path d="M-22 5L-16 19M22 5L16 19" fill="none" />
				<rect x="-4" y="-8" width="8" height="6" rx="1" fill={p.sclera} />
			</>
		)),
		headwear("trail-beanie", "Trail beanie", (p) => (
			<>
				<path d="M-22 0V-7Q-22 -23 0 -23Q22 -23 22 -7V0Z" />
				<path
					d="M-19 -13H19M-22 -7H22"
					fill="none"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<rect x="-23" y="-3" width="46" height="8" rx="3" />
				<path d="M-6 2L0 -6L6 2Z" fill={p.sclera} />
				<path d="M-2 -3L0 -6L2 -3Z" fill={p.accent} />
			</>
		)),
		headwear("aviator-cap", "Aviator cap", (p) => (
			<>
				<path d="M-22 15L-25 -3Q-24 -22 0 -22Q24 -22 25 -3L22 15H15L16 -1H-16L-15 15Z" />
				<path
					d="M-20 -2Q0 -8 20 -2M0 -21V-5"
					fill="none"
					stroke={p.sclera}
					strokeWidth="3"
				/>
				<circle cx="-20" cy="10" r="2" fill={p.accent} />
				<circle cx="20" cy="10" r="2" fill={p.accent} />
			</>
		)),
		eyewear("ski-goggles", "Ski goggles", (c) => {
			const { left, right, radius } = c.anchors.eyes;
			const y = (left[1] + right[1]) / 2;
			const mid = (left[0] + right[0]) / 2;
			return (
				<>
					<path
						d={`M${left[0] - radius - 4} ${y - radius - 2}Q${mid} ${y - radius - 6} ${right[0] + radius + 4} ${y - radius - 2}L${right[0] + radius + 2} ${y + radius + 3}H${mid + 4}L${mid} ${y + 2}L${mid - 4} ${y + radius + 3}H${left[0] - radius - 2}Z`}
						fill={c.palette.clothing}
						stroke={c.palette.accent}
						strokeWidth="2.5"
					/>
					<path
						d={`M${left[0] - 2} ${y - 2}l6 -2M${right[0] - 4} ${y + 1}l6 -2`}
						stroke={c.palette.sclera}
						strokeWidth="2"
					/>
				</>
			);
		}),
		eyewear("aviator-goggles", "Aviator goggles", (c) => (
			<LensPair context={c}>
				<path
					d="M-7 -4Q0 -8 7 -4V1Q3 11 -3 6Q-7 3 -7 -4Z"
					stroke={c.palette.clothing}
					strokeWidth="3"
				/>
				<path d="M-4 -2L0 -3" stroke={c.palette.sclera} />
			</LensPair>
		)),
	],
	arcade: [
		headwear("propeller-cap", "Propeller cap", (p) => (
			<>
				<path d="M-23 2Q-23 -17 0 -17Q23 -17 23 2Z" fill={p.accent} />
				<path d="M0 -17Q-10 -11 -10 2H10Q10 -11 0 -17Z" />
				<path d="M-23 2H27L25 6H-20Z" />
				<path d="M0 -17V-23" fill="none" />
				<ellipse cx="0" cy="-23" rx="17" ry="2.5" fill={p.cheek} />
			</>
		)),
		headwear("pixel-crown", "Pixel crown", (p) => (
			<>
				<path
					d="M-22 3V-17H-15V-10H-7V-22H7V-10H15V-17H22V3Z"
					fill={p.accent}
				/>
				<path d="M-22 -3H22" fill="none" />
				<path
					d="M-3 -13H3V-7H-3ZM-15 -7H-11V-3H-15ZM11 -7H15V-3H11Z"
					fill={p.cheek}
					stroke="none"
				/>
			</>
		)),
		eyewear("pixel-shades", "Pixel shades", (c) => (
			<LensPair context={c}>
				<path
					d="M-9 -5H9V3H5V7H-5V3H-9Z"
					fill={c.palette.ink}
					strokeWidth="1"
				/>
				<path
					d="M-6 -3H-3V0H-6ZM0 0H3V3H0Z"
					fill={c.palette.sclera}
					stroke="none"
				/>
			</LensPair>
		)),
	],
	pirates: [
		headwear("pirate-tricorn", "Pirate tricorn", (p) => (
			<>
				<path d="M-18 -2Q-21 -21 0 -21Q21 -21 18 -2Z" />
				<path d="M-29 -13L-21 7Q0 -1 21 7L29 -13L14 -6L0 -16L-14 -6Z" />
				<path
					d="M-25 -10L-19 3L0 -4L19 3L25 -10"
					fill="none"
					stroke={p.accent}
					strokeWidth="2"
				/>
				<path d="M-4 -10H4L5 -6L2 -3H-2L-5 -6Z" fill={p.sclera} />
				<path d="M-2 -7H-1M1 -7H2" />
			</>
		)),
		headwear("pirate-headscarf", "Pirate headscarf", (p) => (
			<>
				<path d="M-23 3Q-25 -20 0 -20Q25 -20 23 3Z" fill={p.cheek} />
				<path d="M-22 -2Q0 -10 22 -2V4Q0 -3 -22 4Z" />
				<path d="M-23 1L-29 8L-21 10L-26 22L-15 17L-17 2Z" fill={p.cheek} />
				<g fill={p.sclera} stroke="none">
					<circle cx="-12" cy="-9" r="2" />
					<circle cx="0" cy="-14" r="2" />
					<circle cx="12" cy="-9" r="2" />
				</g>
			</>
		)),
		eyewear(
			"pirate-eyepatch",
			"Pirate eyepatch",
			({ anchors: { eyes }, palette }) => (
				<>
					<path
						d={`M${eyes.left[0] - eyes.radius - 7} ${eyes.left[1] - 5}L${eyes.right[0] + eyes.radius + 7} ${eyes.right[1] + 3}`}
						stroke={palette.clothing}
						strokeWidth="2"
					/>
					<path
						d={`M${eyes.left[0] - eyes.radius - 2} ${eyes.left[1] - eyes.radius - 2}h${eyes.radius * 2 + 4}v${eyes.radius + 3}q-${eyes.radius + 2} ${eyes.radius * 2 + 5} -${eyes.radius * 2 + 4} 0Z`}
						fill={palette.ink}
					/>
					<path
						d={`M${eyes.left[0] - 2} ${eyes.left[1] - 2}l4 3M${eyes.left[0] + 2} ${eyes.left[1] - 2}l-4 3`}
						stroke={palette.sclera}
						strokeWidth="1"
					/>
				</>
			),
		),
	],
	arcane: [
		headwear("wizard-hat", "Wizard hat", (p) => (
			<>
				<path d="M-19 0L-2 -24L12 -20L7 -17L19 0Z" />
				<ellipse cx="0" cy="2" rx="28" ry="5" />
				<path d="M-13 -7H13L16 -2H-16Z" fill={p.accent} />
				<path
					d="M0 -19L1 -15L5 -14L1 -13L0 -9L-1 -13L-5 -14L-1 -15Z"
					fill={p.sclera}
					stroke="none"
				/>
			</>
		)),
		headwear("moon-circlet", "Moon circlet", (p) => (
			<>
				<path
					d="M-23 1Q0 -7 23 1"
					fill="none"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path
					d="M4 -23Q-12 -21 -7 -10Q-3 -3 7 -7Q-6 -8 4 -23Z"
					fill={p.accent}
				/>
				<path
					d="M-16 -7L-13 -2L-16 3L-19 -2ZM16 -7L19 -2L16 3L13 -2Z"
					fill={p.sclera}
				/>
			</>
		)),
	],
	celebrations: [
		headwear("party-cone", "Party hat", (p) => (
			<>
				<path d="M-17 3L0 -24L17 3Q0 9 -17 3Z" fill={p.accent} />
				<path
					d="M-9 -10L8 -5M-14 -2L14 3M-4 -18L4 -15"
					stroke={p.cheek}
					strokeWidth="4"
				/>
				<circle cx="0" cy="-23" r="3" fill={p.sclera} />
			</>
		)),
		headwear("santa-hat", "Santa hat", (p) => (
			<>
				<path
					d="M-21 1Q-20 -19 -1 -23Q18 -28 25 -8L17 -4Q9 -15 6 -9L18 1Z"
					fill={p.cheek}
				/>
				<rect x="-23" y="-3" width="44" height="9" rx="4" fill={p.sclera} />
				<circle cx="22" cy="-6" r="5" fill={p.sclera} />
			</>
		)),
		headwear("festive-bow", "Festive bow", (p) => (
			<>
				<path
					d="M-4 -7Q-23 -25 -26 -12Q-28 3 -4 -3ZM4 -7Q23 -25 26 -12Q28 3 4 -3Z"
					fill={p.cheek}
				/>
				<path
					d="M-4 -4L-14 10L-5 8L0 13L4 -3L14 10L5 8L0 13Z"
					fill={p.accent}
				/>
				<rect x="-5" y="-10" width="10" height="9" rx="3" fill={p.accent} />
				<path d="M-18 -10L-8 -6M18 -10L8 -6" fill="none" />
			</>
		)),
		headwear("paper-party-crown", "Paper party crown", (p) => (
			<>
				<path
					d="M-24 4V-17L-16 -10L-8 -19L0 -10L8 -19L16 -10L24 -17V4Z"
					fill={p.cheek}
				/>
				<path d="M-16 -10V4M0 -10V4M16 -10V4" fill="none" stroke={p.accent} />
				<path d="M-24 0H24" stroke={p.sclera} />
			</>
		)),
		eyewear("heart-glasses", "Heart glasses", (c) => (
			<LensPair context={c}>
				<path
					d="M0 -4C-8 -13 -13 -2 -6 4L0 9L6 4C13 -2 8 -13 0 -4Z"
					stroke={c.palette.cheek}
					strokeWidth="2.8"
				/>
			</LensPair>
		)),
	],
	service: [
		headwear("captain-hat", "Captain's hat", (p) => (
			<>
				<path d="M-23 -5L-27 -14Q0 -26 27 -14L23 -5Z" fill={p.sclera} />
				<path d="M-22 -6H22V1H-22Z" />
				<path d="M-18 1Q0 13 18 1Z" />
				<path d="M-5 -15L0 -18L5 -15L3 -10H-3Z" fill={p.accent} />
				<path d="M-18 -2H18" stroke={p.accent} strokeWidth="2" />
			</>
		)),
		headwear("rescue-climbing-helmet", "Rescue climbing helmet", (p) => (
			<>
				<path
					d="M-24 2V-6Q-24 -23 0 -23Q24 -23 24 -6V2L16 5H-16Z"
					fill={p.accent}
				/>
				<path
					d="M-14 -13L-10 -17M14 -13L10 -17M-5 -19V-13M5 -19V-13"
					fill="none"
					strokeWidth="3"
				/>
				<path d="M-24 -2H24" />
				<rect x="-6" y="-7" width="12" height="9" rx="2" />
				<circle cx="0" cy="-3" r="3" fill={p.sclera} />
			</>
		)),
	],
	workplace: [
		headwear("flat-cap", "Flat cap", (p) => (
			<>
				<path d="M-23 0Q-28 -13 -8 -20Q10 -23 24 -7L20 1Z" />
				<path d="M-4 -19Q9 -14 11 -3M-23 -1H23L29 5H5Z" fill={p.accent} />
				<path d="M-18 -5L-14 -13M-8 -4L-5 -16" fill="none" opacity=".45" />
			</>
		)),
		eyewear("cat-eye-frames", "Cat-eye frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-10 -7L-7 3Q0 10 7 2L10 -7Q0 -3 -10 -7Z"
					stroke={c.palette.clothing}
					strokeWidth="2.5"
				/>
			</LensPair>
		)),
	],
	healthcare: [
		headwear("clinic-bouffant-cap", "Clinic bouffant cap", (p) => (
			<>
				<path
					d="M-23 3Q-29 -3 -25 -9Q-26 -17 -17 -17Q-10 -25 -3 -22Q6 -27 13 -21Q23 -22 25 -13Q31 -5 24 3Z"
					fill={p.sclera}
				/>
				<path
					d="M-23 1Q0 -5 24 1M-18 -13L-14 -3M-7 -18L-5 -4M6 -18L5 -4M18 -13L15 -3"
					fill="none"
					stroke={p.clothing}
				/>
			</>
		)),
		headwear("dental-headlight", "Dental headlight", (p) => (
			<>
				<path d="M-23 4V-4Q0 -13 23 -4V4Q0 -4 -23 4Z" />
				<rect x="-8" y="-12" width="16" height="15" rx="4" fill={p.accent} />
				<circle cx="0" cy="-5" r="5" fill={p.sclera} />
				<path d="M-2 -7L1 -8" fill="none" stroke={p.accent} strokeWidth="2" />
			</>
		)),
	],
};

const specialtyHeadwearBySeries: Readonly<
	Record<string, readonly BotEquipmentDefinition[]>
> = {
	flow: [
		headwear("flow-knot-band", "Flow knot headband", (p) => (
			<>
				<path d="M-24 4V-3Q0 -12 24 -3V4Q0 -3 -24 4Z" fill={p.accent} />
				<path d="M-4 -7Q-15 -23 -21 -15Q-24 -7 -7 -4ZM4 -7Q15 -23 21 -15Q24 -7 7 -4Z" />
				<rect x="-5" y="-9" width="10" height="9" rx="3" fill={p.sclera} />
				<path d="M-4 -5L-12 10L-5 7L0 11L4 -4" fill={p.accent} />
			</>
		)),
		headwear("flow-fin-crest", "Flow fin crest", (p) => (
			<>
				<path d="M-22 3Q0 -6 22 3" fill="none" strokeWidth="4" />
				<path d="M-14 0L-12 -18L1 -10L14 -23L12 -4L3 1Z" fill={p.accent} />
				<path
					d="M-12 -18L-4 -3L14 -23M-4 -3L3 1"
					fill="none"
					stroke={p.sclera}
				/>
			</>
		)),
		eyewear("node-link-frames", "Node link frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-7 -4L0 -8L7 -4V4L0 8L-7 4Z"
					stroke={c.palette.clothing}
					strokeWidth="2.2"
				/>
				<rect
					x="-2"
					y="-9"
					width="4"
					height="3"
					rx="1"
					fill={c.palette.accent}
				/>
				<rect
					x="-2"
					y="6"
					width="4"
					height="3"
					rx="1"
					fill={c.palette.accent}
				/>
			</LensPair>
		)),
	],
	basics: [
		headwear("winter-earmuffs", "Winter earmuffs", (p, fit) => {
			const left = Math.min(-25, fit.faceLeft - 8);
			const right = Math.max(25, fit.faceRight + 8);
			return (
				<>
					<path
						d={`M${left} ${fit.earY}V-2Q${left} -19 0 -19Q${right} -19 ${right} -2V${fit.earY}`}
						fill="none"
						strokeWidth="4"
					/>
					<ellipse cx={left} cy={fit.earY} rx="8" ry="11" fill={p.cheek} />
					<ellipse cx={right} cy={fit.earY} rx="8" ry="11" fill={p.cheek} />
					<path
						d={`M${left - 3} ${fit.earY - 6}L${left + 2} ${fit.earY + 4}M${right - 3} ${fit.earY - 5}L${right + 3} ${fit.earY + 4}`}
						stroke={p.sclera}
						opacity=".7"
					/>
				</>
			);
		}),
		headwear("sleeping-cap", "Sleeping cap", (p) => (
			<>
				<path
					d="M-21 2Q-15 -25 9 -21Q24 -18 24 -4L16 -4Q15 -13 7 -10L17 2Z"
					fill={p.cheek}
				/>
				<path
					d="M-15 -6L7 -12M-9 -14L8 -19"
					stroke={p.sclera}
					strokeWidth="3"
				/>
				<rect x="-23" y="-2" width="42" height="8" rx="3" fill={p.sclera} />
				<circle cx="21" cy="0" r="5" fill={p.accent} />
			</>
		)),
		eyewear("octagon-frames", "Octagon frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-3 -7H3L7 -3V3L3 7H-3L-7 3V-3Z"
					stroke={c.palette.accent}
					strokeWidth="1.8"
				/>
				<path d="M-3 -5H1" stroke={c.palette.sclera} />
			</LensPair>
		)),
	],
	nature: [
		headwear("pinecone-crown", "Pinecone crown", (p) => (
			<>
				<path
					d="M-22 3Q0 -5 22 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="4"
				/>
				<path d="M-12 -1L-16 -10L-9 -8L-10 -17L-4 -14L0 -24L4 -14L10 -17L9 -8L16 -10L12 -1Z" />
				<path
					d="M-9 -8L0 -3L9 -8M-4 -14L0 -10L4 -14"
					fill="none"
					stroke={p.accent}
				/>
			</>
		)),
		headwear("butterfly-hairclip", "Butterfly hair clip", (p) => (
			<>
				<path d="M-23 3Q-9 -3 5 -2" fill="none" strokeWidth="3" />
				<path
					d="M5 -7Q-12 -24 -14 -11Q-16 -3 1 -4Q-10 6 -2 9Q5 10 7 -2Q10 9 18 6Q25 0 12 -4Q28 -13 18 -19Q10 -23 5 -7Z"
					fill={p.cheek}
				/>
				<path d="M6 -9L7 3M5 -10L1 -14M6 -10L11 -15" fill="none" />
				<path d="M-9 -12L0 -7M17 -14L11 -8" stroke={p.accent} strokeWidth="3" />
			</>
		)),
		eyewear("leaf-frames", "Leaf frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-9 0Q-4 -11 8 -6Q10 6 -3 7Z"
					stroke={c.palette.accent}
					strokeWidth="2.4"
				/>
				<path d="M-8 0L-11 2M7 -5L9 -7" />
			</LensPair>
		)),
	],
	pets: [
		headwear("bunny-ear-band", "Bunny ear headband", (p) => (
			<>
				<path d="M-23 5Q0 -7 23 5" fill="none" strokeWidth="4" />
				<path
					d="M-15 0Q-23 -21 -15 -24Q-7 -25 -6 -2ZM6 -2Q6 -24 14 -24Q24 -22 16 0Z"
					fill={p.sclera}
				/>
				<path
					d="M-14 -17L-11 -5M13 -17L11 -5"
					stroke={p.cheek}
					strokeWidth="4"
				/>
			</>
		)),
		headwear("hamster-hood", "Hamster hood", (p) => (
			<>
				<circle cx="-18" cy="-12" r="8" fill={p.accent} />
				<circle cx="18" cy="-12" r="8" fill={p.accent} />
				<circle cx="-18" cy="-12" r="4" fill={p.cheek} />
				<circle cx="18" cy="-12" r="4" fill={p.cheek} />
				<path
					d="M-24 16V-3Q-23 -14 0 -14Q23 -14 24 -3V16L18 21L17 -1Q0 -8 -17 -1L-18 21Z"
					fill={p.accent}
				/>
				<path d="M-20 9L-26 8M20 9L26 8" stroke={p.sclera} />
			</>
		)),
		eyewear("bone-bridge-frames", "Bone frames", (c) => (
			<LensPair context={c}>
				<rect
					x="-7"
					y="-5"
					width="14"
					height="11"
					rx="4"
					stroke={c.palette.clothing}
					strokeWidth="2"
				/>
				<path
					d="M-5 -5Q-9 -9 -10 -5Q-11 -1 -7 -1M5 -5Q9 -9 10 -5Q11 -1 7 -1"
					fill={c.palette.sclera}
				/>
			</LensPair>
		)),
	],
	zoo: [
		headwear("giraffe-headband", "Giraffe headband", (p) => (
			<>
				<path
					d="M-22 3Q0 -8 22 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="5"
				/>
				<path
					d="M-12 -3L-14 -15M12 -3L14 -15"
					stroke={p.accent}
					strokeWidth="6"
				/>
				<circle cx="-14" cy="-18" r="4" />
				<circle cx="14" cy="-18" r="4" />
				<path
					d="M-18 -1Q-31 -12 -31 -2Q-28 5 -18 -1ZM18 -1Q31 -12 31 -2Q28 5 18 -1Z"
					fill={p.cheek}
				/>
			</>
		)),
		headwear("elephant-ear-band", "Elephant ear headband", (p) => (
			<>
				<path d="M-22 2Q0 -9 22 2" fill="none" strokeWidth="4" />
				<path d="M-18 -1Q-32 -13 -33 1Q-35 20 -24 23Q-16 24 -14 8ZM18 -1Q32 -13 33 1Q35 20 24 23Q16 24 14 8Z" />
				<path
					d="M-25 1Q-31 12 -24 17M25 1Q31 12 24 17"
					stroke={p.cheek}
					strokeWidth="5"
					fill="none"
				/>
			</>
		)),
		eyewear("butterfly-frames", "Butterfly frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-9 -7Q0 -11 7 -5L5 5Q0 9 -5 5Z"
					stroke={c.palette.cheek}
					strokeWidth="2.6"
				/>
				<path
					d="M-9 -7L-11 -10M7 -5L10 -8"
					stroke={c.palette.accent}
					strokeWidth="2"
				/>
			</LensPair>
		)),
	],
	tech: [
		headwear("coil-cap", "Coil cap", (p) => (
			<>
				<path d="M-21 2Q-22 -11 0 -11Q22 -11 21 2Z" />
				<path d="M-6 -11V-23H6V-11" fill={p.sclera} />
				<path
					d="M-9 -12H9M-10 -16H10M-9 -20H9"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<circle cx="0" cy="-24" r="2" fill={p.accent} />
				<path d="M-16 -2H-10M10 -2H16" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		eyewear("ar-monocle", "AR monocle", ({ anchors: { eyes }, palette }) => (
			<g
				transform={`translate(${eyes.right[0]} ${eyes.right[1]}) scale(${(eyes.radius + 3) / 8})`}
			>
				<path
					d="M-9 -7H7L11 -2V6H-9Z"
					stroke={palette.accent}
					strokeWidth="2"
				/>
				<path d="M-6 -4H0M-6 -1H-2M3 3H7M11 -1H16" stroke={palette.clothing} />
				<circle cx="8" cy="-5" r="1.5" fill={palette.cheek} stroke="none" />
			</g>
		)),
		eyewear("scanline-glasses", "Scanline glasses", (c) => (
			<LensPair context={c}>
				<rect
					x="-8"
					y="-5"
					width="16"
					height="10"
					rx="1.5"
					fill={c.palette.clothing}
				/>
				<path
					d="M-5 -2H5M-5 1H5M-5 4H5"
					stroke={c.palette.accent}
					strokeWidth="1"
				/>
			</LensPair>
		)),
	],
	snacks: [
		headwear("soft-serve-hat", "Soft serve hat", (p) => (
			<>
				<path
					d="M-19 2Q-25 -5 -14 -9Q-18 -16 -5 -18Q-7 -24 4 -25Q1 -20 11 -18Q22 -14 15 -9Q26 -3 19 2Z"
					fill={p.sclera}
				/>
				<path
					d="M-14 -9Q0 -4 15 -9M-5 -18Q2 -13 11 -18"
					stroke={p.cheek}
					strokeWidth="2.5"
					fill="none"
				/>
				<path d="M-19 2H19L15 8H-15Z" fill={p.accent} />
			</>
		)),
		headwear("croissant-headband", "Croissant headband", (p) => (
			<>
				<path d="M-22 3Q0 -6 22 3" fill="none" strokeWidth="3" />
				<path
					d="M-24 -1Q-27 -12 -17 -18Q-7 -25 0 -20Q7 -25 17 -18Q27 -12 24 -1L14 -5Q0 -11 -14 -5Z"
					fill={p.accent}
				/>
				<path
					d="M-17 -18L-13 -6M-8 -21L-6 -9M8 -21L6 -9M17 -18L13 -6"
					fill="none"
				/>
			</>
		)),
		eyewear("pretzel-frames", "Pretzel frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-7 3Q-12 -7 -5 -8Q0 -10 4 -3Q11 -10 10 -2Q10 7 2 7Q-3 8 -7 3Z"
					stroke={c.palette.accent}
					strokeWidth="2.7"
				/>
				<path
					d="M-5 -6L5 5M5 -5L-5 5"
					stroke={c.palette.accent}
					strokeWidth="1.7"
				/>
			</LensPair>
		)),
	],
	cosmos: [
		headwear("comet-crest", "Comet crest", (p) => (
			<>
				<path d="M-22 3Q0 -6 22 3" fill="none" strokeWidth="4" />
				<path
					d="M-12 -3L-26 -15L-14 -12L-20 -24L-6 -17L-8 -25L7 -12Z"
					fill={p.accent}
				/>
				<circle cx="7" cy="-7" r="9" fill={p.cheek} />
				<path
					d="M4 -10L9 -12M-12 -14L-3 -8"
					stroke={p.sclera}
					strokeWidth="2"
				/>
			</>
		)),
		headwear("ufo-hat", "UFO hat", (p) => (
			<>
				<path d="M-13 -5Q-13 -24 0 -24Q13 -24 13 -5Z" fill={p.sclera} />
				<ellipse cx="0" cy="-4" rx="28" ry="7" />
				<path d="M-24 -1Q0 12 24 -1" fill={p.accent} />
				<path
					d="M-6 -19Q-10 -15 -9 -12"
					stroke={p.accent}
					strokeWidth="2"
					fill="none"
				/>
				<g fill={p.cheek} stroke="none">
					<circle cx="-17" cy="-3" r="2" />
					<circle cx="0" cy="-1" r="2" />
					<circle cx="17" cy="-3" r="2" />
				</g>
			</>
		)),
		eyewear("starport-visor", "Starport visor", (c) => (
			<LensPair context={c}>
				<path
					d="M-8 -6H4L9 -1L4 6H-8Z"
					fill={c.palette.clothing}
					stroke={c.palette.accent}
					strokeWidth="2"
				/>
				<path d="M-4 -3L2 -3M-4 1H0" stroke={c.palette.sclera} />
			</LensPair>
		)),
	],
	pro: [
		headwear("bowler-hat", "Bowler hat", (p) => (
			<>
				<path d="M-18 0V-6Q-18 -24 0 -24Q18 -24 18 -6V0Z" />
				<path d="M-18 -7H18V-1H-18Z" fill={p.accent} />
				<path d="M-28 -1Q-23 9 -17 5H17Q23 9 28 -1Q0 7 -28 -1Z" />
				<path
					d="M-11 -17Q-6 -21 -1 -20"
					stroke={p.sclera}
					opacity=".6"
					fill="none"
				/>
			</>
		)),
		headwear("feather-fascinator", "Feather fascinator", (p) => (
			<>
				<path d="M-23 4Q0 -8 23 4" fill="none" strokeWidth="3" />
				<ellipse
					cx="8"
					cy="-5"
					rx="19"
					ry="6"
					transform="rotate(-15 8 -5)"
					fill={p.accent}
				/>
				<path
					d="M6 -6Q-6 -22 6 -23Q17 -21 10 -7M9 -7Q20 -25 24 -17Q29 -8 12 -5"
					fill={p.cheek}
				/>
				<path d="M-10 1L-2 14L7 3M-6 5L4 10" fill="none" stroke={p.sclera} />
			</>
		)),
		eyewear("browline-frames", "Browline frames", (c) => (
			<LensPair context={c}>
				<path d="M-7 -4H7V1Q7 7 0 7Q-7 7 -7 1Z" stroke={c.palette.accent} />
				<path
					d="M-8 -3V-5Q0 -8 8 -5V-3"
					stroke={c.palette.clothing}
					strokeWidth="3"
				/>
			</LensPair>
		)),
	],
	office: [
		headwear("paper-boat-hat", "Paper boat hat", (p) => (
			<>
				<path d="M-25 2L0 -22L25 2Z" fill={p.sclera} />
				<path d="M0 -22V2M-25 2H25L20 8H-20Z" fill={p.sclera} />
				<path
					d="M-12 -5H-4M5 -4H12M-15 4H5"
					stroke={p.clothing}
					strokeWidth="1"
				/>
			</>
		)),
		headwear("binder-clip-hairpiece", "Binder clip hairpiece", (p) => (
			<>
				<path d="M-23 3Q0 -6 23 3" fill="none" strokeWidth="3" />
				<path d="M-12 1L-8 -12H8L12 1Z" />
				<path
					d="M-5 -11L-9 -19Q-9 -24 -4 -24H4Q9 -24 9 -19L5 -11M-7 0L-10 8Q-10 12 -5 12H5Q10 12 10 8L7 0"
					fill="none"
					stroke={p.sclera}
					strokeWidth="2.2"
				/>
				<path d="M-8 -10H8" stroke={p.accent} strokeWidth="2" />
			</>
		)),
		eyewear("pince-nez", "Pince-nez", (c) => {
			const { left, right, radius } = c.anchors.eyes;
			const rx = (6 * (radius + 2.5)) / 7;
			const mid = (left[0] + right[0]) / 2;
			const top = Math.min(left[1], right[1]) - 10;
			return (
				<>
					<LensPair context={c} bridge={false}>
						<ellipse
							cx="0"
							cy="0"
							rx="6"
							ry="7"
							stroke={c.palette.accent}
							strokeWidth="1.6"
						/>
					</LensPair>
					<path
						d={`M${left[0] + rx} ${left[1] - 2}Q${mid} ${top} ${right[0] - rx} ${right[1] - 2}`}
						stroke={c.palette.accent}
					/>
				</>
			);
		}),
	],
	workshop: [
		headwear("lifted-welding-helmet", "Lifted welding helmet", (p) => (
			<>
				<path d="M-23 8V-8Q-20 -17 0 -17Q20 -17 23 -8V8H17V-2H-17V8Z" />
				<path d="M-20 -23H20L16 0H-16Z" fill={p.accent} />
				<rect x="-13" y="-18" width="26" height="9" rx="2" fill={p.ink} />
				<path d="M-8 -14H7" stroke={p.sclera} strokeWidth="2" />
				<circle cx="-20" cy="1" r="2" fill={p.sclera} />
				<circle cx="20" cy="1" r="2" fill={p.sclera} />
			</>
		)),
		headwear("ear-defenders", "Ear defenders", (p, fit) => {
			const left = Math.min(-25, fit.faceLeft - 6);
			const right = Math.max(25, fit.faceRight + 6);
			return (
				<>
					<path
						d={`M${left} ${fit.earY}V-3Q${left} -20 0 -20Q${right} -20 ${right} -3V${fit.earY}`}
						fill="none"
						stroke={p.accent}
						strokeWidth="6"
					/>
					<rect
						x={left - 6}
						y={fit.earY - 11.5}
						width="12"
						height="23"
						rx="5"
					/>
					<rect
						x={right - 6}
						y={fit.earY - 11.5}
						width="12"
						height="23"
						rx="5"
					/>
					<path
						d={`M${left} ${fit.earY - 6}V${fit.earY + 6}M${right} ${fit.earY - 6}V${fit.earY + 6}`}
						stroke={p.accent}
						strokeWidth="3"
					/>
				</>
			);
		}),
		eyewear(
			"jeweler-loupe",
			"Jeweler's loupe",
			({ anchors: { eyes }, palette }) => (
				<g
					transform={`translate(${eyes.left[0]} ${eyes.left[1]}) scale(${(eyes.radius + 2) / 7})`}
				>
					<circle cx="0" cy="0" r="8" stroke={palette.accent} strokeWidth="3" />
					<circle cx="0" cy="0" r="5" stroke={palette.clothing} />
					<path d="M-8 -2H-13M6 -6L12 -11" strokeWidth="2" />
					<circle cx="13" cy="-12" r="4" fill={palette.sclera} />
				</g>
			),
		),
	],
	ocean: [
		headwear("diving-hood", "Diving hood", (p, fit) => {
			const left = Math.min(-18, fit.faceLeft);
			const right = Math.max(18, fit.faceRight);
			return (
				<>
					<path
						d={`M${left - 8} 21V-1Q${left - 8} -24 0 -24Q${right + 8} -24 ${right + 8} -1V21L${right} 25V-2Q0 -10 ${left} -2V25Z`}
					/>
					<path
						d={`M0 -22V-6M${left - 3} 13V21M${right + 3} 13V21`}
						stroke={p.accent}
						strokeWidth="3"
					/>
					<path
						d={`M${left} -3Q0 -11 ${right} -3`}
						fill="none"
						stroke={p.sclera}
						strokeWidth="2"
					/>
				</>
			);
		}),
		eyewear("snorkel-mask", "Snorkel mask", (c) => {
			const { left, right, radius } = c.anchors.eyes;
			const y = (left[1] + right[1]) / 2;
			const x = left[0] - radius - 5;
			const width = right[0] - left[0] + radius * 2 + 10;
			return (
				<>
					<rect
						x={x}
						y={y - radius - 3}
						width={width}
						height={radius * 2 + 7}
						rx="7"
						stroke={c.palette.accent}
						strokeWidth="3"
					/>
					<path
						d={`M${(left[0] + right[0]) / 2} ${y - radius - 3}v${radius * 2 + 7}M${x + width + 3} ${y + 11}q7 0 7 -6v-24q0 -5 -5 -5h-3`}
						stroke={c.palette.clothing}
						strokeWidth="3"
					/>
					<path
						d={`M${x + 6} ${y - radius + 1}h8`}
						stroke={c.palette.sclera}
						strokeWidth="2"
					/>
				</>
			);
		}),
		eyewear("porthole-frames", "Porthole frames", (c) => (
			<LensPair context={c}>
				<circle cx="0" cy="0" r="8" stroke={c.palette.accent} strokeWidth="3" />
				<g fill={c.palette.ink} stroke="none">
					<circle cx="0" cy="-8" r="1" />
					<circle cx="8" cy="0" r="1" />
					<circle cx="0" cy="8" r="1" />
					<circle cx="-8" cy="0" r="1" />
				</g>
			</LensPair>
		)),
	],
	fantasy: [
		headwear("winged-helmet", "Winged helmet", (p) => (
			<>
				<path
					d="M-19 4V-5Q-19 -21 0 -21Q19 -21 19 -5V4L12 8V0H-12V8Z"
					fill={p.sclera}
				/>
				<path
					d="M-17 -3Q-27 -4 -24 -18L-20 -12L-18 -20L-14 -12L-11 -16L-13 -5ZM17 -3Q27 -4 24 -18L20 -12L18 -20L14 -12L11 -16L13 -5Z"
					fill={p.accent}
				/>
				<path d="M0 -20V1" strokeWidth="3" />
			</>
		)),
		headwear("unicorn-tiara", "Unicorn tiara", (p) => (
			<>
				<path
					d="M-23 3L-16 -6L-9 1L0 -5L9 1L16 -6L23 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path d="M-6 -3L0 -25L7 -3Z" fill={p.sclera} />
				<path d="M-3 -15L3 -18M-5 -8L5 -12" stroke={p.cheek} strokeWidth="2" />
				<circle cx="-16" cy="-6" r="2" fill={p.cheek} />
				<circle cx="16" cy="-6" r="2" fill={p.cheek} />
			</>
		)),
		eyewear("dragon-eye-mask", "Dragon eye mask", (c) => (
			<LensPair context={c}>
				<path
					d="M-10 -8L-3 -5L2 -9L10 -4L7 7L0 5L-7 8ZM-5 -1L0 -3L5 -1L3 3H-3Z"
					fill={c.palette.clothing}
					fillRule="evenodd"
				/>
				<path
					d="M-6 -6L-5 -10M5 -6L7 -10"
					stroke={c.palette.accent}
					strokeWidth="2"
				/>
			</LensPair>
		)),
	],
	garden: [
		headwear("bee-antenna-band", "Bee antenna headband", (p) => (
			<>
				<path
					d="M-23 3Q0 -8 23 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="5"
				/>
				<path
					d="M-11 -3Q-18 -10 -13 -17M11 -3Q18 -10 13 -17"
					fill="none"
					strokeWidth="3"
				/>
				<circle cx="-13" cy="-19" r="5" />
				<circle cx="13" cy="-19" r="5" />
				<path d="M-17 -19H-9M9 -19H17" stroke={p.accent} strokeWidth="2.5" />
				<path d="M-16 2L-13 -4M-4 0L0 -6M9 0L13 -4" />
			</>
		)),
		headwear("tulip-hat", "Tulip hat", (p) => (
			<>
				<path
					d="M-19 3Q-26 -5 -21 -21L-10 -11L0 -24L10 -11L21 -21Q26 -5 19 3Z"
					fill={p.cheek}
				/>
				<path
					d="M-10 -11Q-6 -3 0 3Q6 -3 10 -11M-19 3Q0 -1 19 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="2"
				/>
				<path d="M-20 3L-11 8L0 3L11 8L20 3" fill={p.accent} />
			</>
		)),
		eyewear("flower-frames", "Flower frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-4 -6Q-3 -12 2 -9Q8 -11 8 -5Q14 -2 8 3Q10 9 3 9Q0 14 -4 8Q-11 10 -9 3Q-14 -1 -8 -5Q-10 -11 -4 -6Z"
					stroke={c.palette.cheek}
					strokeWidth="2.3"
				/>
				<circle cx="0" cy="0" r="5" stroke={c.palette.accent} />
			</LensPair>
		)),
	],
	music: [
		headwear("drum-major-shako", "Drum major shako", (p) => (
			<>
				<path d="M-18 1L-16 -20H16L18 1Z" />
				<path d="M-16 -13H16M-17 -5H17" stroke={p.accent} strokeWidth="3" />
				<path d="M-17 1Q0 10 17 1Z" fill={p.accent} />
				<path d="M2 -20Q-8 -30 -10 -20Q-11 -12 -3 -12L-1 -20" fill={p.sclera} />
				<path d="M-4 -10L0 -15L4 -10L0 -6Z" fill={p.sclera} />
			</>
		)),
		headwear("record-headband", "Record headband", (p) => (
			<>
				<path
					d="M-23 3Q0 -6 23 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="4"
				/>
				{[-16, 0, 16].map((x) => (
					<g key={x}>
						<circle cx={x} cy={x ? -5 : -14} r="8" fill={p.ink} />
						<circle
							cx={x}
							cy={x ? -5 : -14}
							r="5.5"
							fill="none"
							stroke={p.clothing}
						/>
						<circle
							cx={x}
							cy={x ? -5 : -14}
							r="2.5"
							fill={p.cheek}
							stroke="none"
						/>
					</g>
				))}
			</>
		)),
		eyewear(
			"treble-clef-monocle",
			"Treble clef monocle",
			({ anchors: { eyes }, palette }) => (
				<g
					transform={`translate(${eyes.right[0]} ${eyes.right[1]}) scale(${(eyes.radius + 2) / 7})`}
					stroke={palette.accent}
				>
					<circle cx="0" cy="0" r="7" strokeWidth="2" />
					<path
						d="M0 11Q-4 15 -5 11Q-5 8 -1 10L-2 -15Q-2 -20 2 -18Q7 -14 -2 -8Q-10 -1 -3 4Q4 7 4 0Q2 -5 -3 -1"
						strokeWidth="1.7"
					/>
				</g>
			),
		),
	],
	adventure: [
		headwear("cowboy-hat", "Cowboy hat", (p) => (
			<>
				<path
					d="M-18 0L-14 -20Q-7 -24 0 -17Q7 -24 14 -20L18 0Z"
					fill={p.accent}
				/>
				<path d="M-17 -6Q0 -2 17 -6V0H-17Z" />
				<path
					d="M-30 -7Q-24 11 -16 5H16Q24 11 30 -7Q24 0 18 -1H-18Q-24 0 -30 -7Z"
					fill={p.accent}
				/>
				<path d="M-3 -4H3V1H-3Z" fill={p.sclera} />
			</>
		)),
		headwear("cycling-helmet", "Cycling helmet", (p) => (
			<>
				<path
					d="M-25 2Q-27 -17 -4 -22Q20 -25 26 -3L20 4H-18Z"
					fill={p.sclera}
				/>
				<path
					d="M-18 -8L-9 -14M-7 -11L2 -17M5 -10L14 -15M15 -6L21 -8"
					stroke={p.clothing}
					strokeWidth="4"
				/>
				<path d="M-19 3L-12 20L-3 23M19 3L12 20L3 23" fill="none" />
				<path d="M-23 0H24" stroke={p.accent} strokeWidth="3" />
			</>
		)),
		eyewear("wraparound-sport-shades", "Wraparound sport shades", (c) => {
			const { left, right, radius } = c.anchors.eyes;
			const y = Math.min(left[1], right[1]);
			const mid = (left[0] + right[0]) / 2;
			return (
				<>
					<path
						d={`M${left[0] - radius - 5} ${y - 4}Q${mid} ${y - radius - 6} ${right[0] + radius + 5} ${y - 4}L${right[0] + radius} ${y + 5}L${mid + 4} ${y + 4}L${mid} ${y}L${mid - 4} ${y + 4}L${left[0] - radius} ${y + 5}Z`}
						fill={c.palette.clothing}
						stroke={c.palette.accent}
						strokeWidth="2"
					/>
					<path
						d={`M${left[0] - 4} ${y - 1}l6 -2M${right[0] - 4} ${y - 1}l6 -2`}
						stroke={c.palette.sclera}
					/>
				</>
			);
		}),
		headwear("tennis-visor", "Tennis visor", (p) => (
			<>
				<path d="M-22 4V-3Q0 -13 22 -3V4Q0 -5 -22 4Z" fill={p.sclera} />
				<path d="M-21 2Q2 -6 26 3L23 10Q0 4 -21 8Z" fill={p.accent} />
				<path d="M-5 -6H5M-2 -8V-3" stroke={p.clothing} strokeWidth="2" />
			</>
		)),
		headwear("ski-earflap-hat", "Ski earflap hat", (p) => (
			<>
				<path d="M-24 16V-4Q-23 -21 0 -21Q23 -21 24 -4V16L15 19L12 4H-12L-15 19Z" />
				<path d="M-23 -4H23V4H-23Z" fill={p.sclera} />
				<path
					d="M-15 -8L-8 -14L0 -8L8 -14L15 -8"
					stroke={p.accent}
					strokeWidth="3"
					fill="none"
				/>
				<circle cx="0" cy="-23" r="4" fill={p.cheek} />
				<path d="M-19 18V27M19 18V27" stroke={p.cheek} strokeWidth="3" />
			</>
		)),
		eyewear(
			"fencing-mask",
			"Fencing mask",
			({ anchors: { eyes }, palette }) => {
				const cx = (eyes.left[0] + eyes.right[0]) / 2;
				const cy = (eyes.left[1] + eyes.right[1]) / 2;
				const w = eyes.right[0] - eyes.left[0] + eyes.radius * 2 + 10;
				const h = eyes.radius * 2 + 13;
				return (
					<g transform={`translate(${cx} ${cy}) scale(${w / 40} ${h / 30})`}>
						<path
							d="M-20 -8Q-20 -16 0 -16Q20 -16 20 -8V7Q16 17 0 17Q-16 17 -20 7Z"
							stroke={palette.clothing}
							strokeWidth="3"
						/>
						<path
							d="M-13 -12V12M-6 -14V15M1 -14V15M8 -14V14M15 -11V10M-17 -7H17M-18 0H18M-17 7H17"
							stroke={palette.accent}
							strokeWidth=".8"
						/>
					</g>
				);
			},
		),
	],
	arcade: [
		headwear("chess-rook-hat", "Chess rook hat", (p) => (
			<>
				<path
					d="M-20 -21H-12V-14H-5V-21H5V-14H12V-21H20V-7L15 -3L17 5H-17L-15 -3L-20 -7Z"
					fill={p.sclera}
				/>
				<path d="M-18 -8H18M-15 -2H15" stroke={p.clothing} strokeWidth="2" />
				<rect x="-21" y="4" width="42" height="5" rx="2" fill={p.accent} />
			</>
		)),
		eyewear("checkerboard-frames", "Checkerboard frames", (c) => (
			<LensPair context={c}>
				<rect
					x="-8"
					y="-6"
					width="16"
					height="13"
					rx="1"
					stroke={c.palette.sclera}
					strokeWidth="3"
				/>
				<path
					d="M-8 -6H-4M0 -6H4M8 -2V2M4 7H0M-4 7H-8M-8 2V-2"
					stroke={c.palette.ink}
					strokeWidth="3"
				/>
			</LensPair>
		)),
		eyewear(
			"cinema-3d-glasses",
			"3D cinema glasses",
			({ anchors: { eyes }, palette }) => (
				<>
					<path
						d={`M${eyes.left[0] - eyes.radius - 5} ${eyes.left[1]}H${eyes.right[0] + eyes.radius + 5}`}
						strokeWidth="3"
					/>
					{(["left", "right"] as const).map((side) => (
						<g
							key={side}
							transform={`translate(${eyes[side][0]} ${eyes[side][1]}) scale(${(eyes.radius + 2) / 7})`}
						>
							<rect
								x="-8"
								y="-6"
								width="16"
								height="12"
								rx="1"
								fill={side === "left" ? palette.cheek : palette.accent}
								fillOpacity=".65"
								stroke={palette.sclera}
								strokeWidth="3"
							/>
							<path d="M-4 -3L0 -4" stroke={palette.sclera} />
						</g>
					))}
				</>
			),
		),
	],
	pirates: [
		headwear("pirate-bicorn", "Pirate bicorn", (p) => (
			<>
				<path d="M-15 1Q-19 -20 0 -20Q19 -20 15 1Z" />
				<path d="M-30 -10Q0 1 30 -10L22 6Q0 0 -22 6Z" />
				<path
					d="M-25 -5Q0 5 25 -5"
					stroke={p.accent}
					strokeWidth="2"
					fill="none"
				/>
				<path d="M-5 -13L5 -7M5 -13L-5 -7" stroke={p.sclera} strokeWidth="2" />
			</>
		)),
		headwear("parrot-feather-band", "Parrot feather headband", (p) => (
			<>
				<path
					d="M-23 4Q0 -5 23 4"
					stroke={p.accent}
					strokeWidth="4"
					fill="none"
				/>
				<path
					d="M-9 0Q-28 -8 -23 -20Q-12 -18 -6 -2M-6 -1Q-16 -25 -5 -25Q5 -21 0 -2M-1 0Q4 -22 14 -20Q17 -9 3 2"
					fill={p.cheek}
				/>
				<path d="M-20 -16L-8 -2M-5 -20L-4 -3M10 -15L1 1" stroke={p.sclera} />
			</>
		)),
		eyewear(
			"navigator-sighting-lens",
			"Navigator's sighting lens",
			({ anchors: { eyes }, palette }) => (
				<g
					transform={`translate(${eyes.right[0]} ${eyes.right[1]}) scale(${(eyes.radius + 2) / 7})`}
				>
					<circle
						cx="0"
						cy="0"
						r="8"
						stroke={palette.accent}
						strokeWidth="2.5"
					/>
					<path d="M-11 0H-5M5 0H11M0 -11V-5M0 5V11M-8 -8L-12 -12" />
					<path d="M3 7L9 18H4L-2 8" fill={palette.accent} />
				</g>
			),
		),
	],
	arcane: [
		headwear("crystal-circlet", "Crystal circlet", (p) => (
			<>
				<path
					d="M-23 3Q0 -6 23 3"
					fill="none"
					stroke={p.accent}
					strokeWidth="3"
				/>
				<path
					d="M-19 0L-22 -13L-15 -18L-10 -9L-9 -2M-6 -3L-7 -17L0 -25L7 -17L6 -3M9 -2L10 -9L15 -18L22 -13L19 0Z"
					fill={p.cheek}
				/>
				<path d="M0 -25V-3M-15 -18L-14 -1M15 -18L14 -1" stroke={p.sclera} />
			</>
		)),
		headwear("star-wrap", "Star wrap", (p) => (
			<>
				<path d="M-23 3Q-27 -17 -9 -21Q6 -27 21 -12Q26 -7 23 3Z" />
				<path
					d="M-21 0Q-12 -13 11 -20M-17 -12Q0 -7 20 3M-24 -4Q-3 3 17 -9"
					stroke={p.accent}
					strokeWidth="3"
					fill="none"
				/>
				<path
					d="M2 -8L4 -4L8 -3L5 0L5 4L1 2L-3 4L-2 0L-5 -3L0 -4Z"
					fill={p.sclera}
				/>
			</>
		)),
		eyewear("alchemy-spectacles", "Alchemy spectacles", (c) => (
			<LensPair context={c}>
				<circle cx="0" cy="0" r="7" stroke={c.palette.accent} strokeWidth="2" />
				<path d="M-7 -4L0 -9L7 -4M-6 5L0 9L6 5" stroke={c.palette.clothing} />
				<path d="M-1 -3L2 1H-4Z" stroke={c.palette.sclera} strokeWidth=".8" />
			</LensPair>
		)),
	],
	celebrations: [
		headwear("ruffled-bonnet", "Ruffled bonnet", (p, fit) => {
			const left = Math.min(-17, fit.faceLeft);
			const right = Math.max(17, fit.faceRight);
			return (
				<>
					<path
						d={`M${left - 8} 13Q${left - 14} 5 ${left - 9} 0Q${left - 13} -8 ${left - 4} -12Q${left - 3} -21 -10 -19Q-3 -27 5 -21Q16 -25 ${right + 3} -15Q${right + 13} -13 ${right + 9} -3Q${right + 15} 3 ${right + 8} 12L${right} 7Q${right + 3} -14 0 -14Q${left - 3} -14 ${left} 7Z`}
						fill={p.sclera}
					/>
					<path
						d={`M${left - 9} 8L${left - 7} 23L${left} 18L${left - 2} 8M${right + 9} 8L${right + 7} 23L${right} 18L${right + 2} 8`}
						fill={p.cheek}
					/>
					<path
						d={`M${left - 1} -9Q0 -22 ${right + 1} -9`}
						fill="none"
						stroke={p.accent}
						strokeWidth="2"
					/>
				</>
			);
		}),
		headwear("birthday-candle-crown", "Birthday candle crown", (p) => (
			<>
				<path d="M-23 4V-3Q0 -10 23 -3V4Z" fill={p.cheek} />
				<path
					d="M-17 -3V-14H-11V-3M-3 -5V-19H3V-5M11 -3V-14H17V-3"
					fill={p.sclera}
				/>
				<path
					d="M-15 -16Q-19 -20 -14 -23Q-9 -18 -15 -16M0 -21Q-4 -25 0 -28Q5 -23 0 -21M14 -16Q10 -20 14 -23Q19 -18 14 -16"
					fill={p.accent}
					stroke="none"
				/>
				<path
					d="M-17 -9H-11M-3 -12H3M11 -9H17"
					stroke={p.cheek}
					strokeWidth="2"
				/>
			</>
		)),
		eyewear("confetti-frames", "Confetti frames", (c) => (
			<LensPair context={c}>
				<path
					d="M-7 -5L-2 -8L3 -6L8 -7L7 0L9 5L3 7L-2 6L-7 8L-6 1Z"
					stroke={c.palette.cheek}
					strokeWidth="2.5"
				/>
				<path
					d="M-10 -9L-12 -12M2 -11V-14M11 0H14M-10 7L-13 10"
					stroke={c.palette.accent}
					strokeWidth="1.5"
				/>
			</LensPair>
		)),
	],
	service: [
		headwear("motorcycle-helmet", "Motorcycle helmet", (p, fit) => {
			const left = Math.min(-17, fit.faceLeft);
			const right = Math.max(17, fit.faceRight);
			const brimBottom = Math.min(5, fit.faceTop);
			return (
				<>
					<path
						d={`M${left - 7} 17L${left - 9} -3Q${left - 7} -24 0 -24Q${right + 7} -24 ${right + 9} -3L${right + 7} 17L${right} 20V${brimBottom}H${left}V20Z`}
						fill={p.sclera}
					/>
					<path
						d={`M${left - 6} -1Q0 -8 ${right + 6} -1L${right + 3} ${brimBottom}H${left - 3}Z`}
						fill={p.clothing}
					/>
					<path
						d={`M-4 -22V-8H4V-22M${left - 5} 12H${left}M${right} 12H${right + 5}`}
						fill={p.accent}
						stroke={p.accent}
						strokeWidth="2"
					/>
				</>
			);
		}),
		eyewear("pilot-sunshield", "Pilot sunshield", (c) => (
			<LensPair context={c}>
				<path
					d="M-8 -4L-3 -7H5L8 -3L4 6H-4Z"
					fill={c.palette.clothing}
					stroke={c.palette.accent}
					strokeWidth="1.7"
				/>
				<path d="M-4 -2L3 -3" stroke={c.palette.sclera} />
			</LensPair>
		)),
		eyewear("rescue-visor", "Rescue visor", (c) => {
			const { left, right, radius } = c.anchors.eyes;
			const y = Math.min(left[1], right[1]);
			const x = left[0] - radius - 5;
			const width = right[0] - left[0] + radius * 2 + 10;
			return (
				<>
					<path
						d={`M${x} ${y - radius - 5}h${width}v${radius * 2 + 8}q-${width / 2} 8 -${width} 0Z`}
						fill={c.palette.sclera}
						fillOpacity=".18"
						stroke={c.palette.accent}
						strokeWidth="2"
					/>
					<path
						d={`M${x} ${y - radius - 5}h${width}M${x + 4} ${y - radius}v8`}
						stroke={c.palette.clothing}
						strokeWidth="3"
					/>
					<circle
						cx={x + 3}
						cy={y - radius - 5}
						r="2"
						fill={c.palette.sclera}
					/>
					<circle
						cx={x + width - 3}
						cy={y - radius - 5}
						r="2"
						fill={c.palette.sclera}
					/>
				</>
			);
		}),
	],
	workplace: [
		headwear("press-visor", "Press visor", (p) => (
			<>
				<path d="M-23 3V-7Q0 -13 23 -7V3Z" />
				<path d="M-23 1H24L30 7H-14Z" fill={p.accent} />
				<path d="M5 -16L19 -14L17 -3L3 -5Z" fill={p.sclera} />
				<path d="M8 -12L15 -11M7 -9L13 -8" strokeWidth="1" />
			</>
		)),
		headwear("coffee-cup-hairclip", "Coffee cup hair clip", (p) => (
			<>
				<path d="M-23 3Q-10 -6 7 0" fill="none" strokeWidth="3" />
				<path d="M15 -14H21Q29 -7 18 -3" fill="none" strokeWidth="3" />
				<path d="M-3 -16H17L15 -3Q7 5 0 -3Z" fill={p.sclera} />
				<ellipse cx="7" cy="-16" rx="10" ry="3" fill={p.clothing} />
				<path
					d="M3 -8Q7 -13 10 -8Q11 -5 7 -3Q2 -5 3 -8Z"
					fill={p.cheek}
					stroke="none"
				/>
				<path d="M-7 3H23" stroke={p.accent} strokeWidth="3" />
			</>
		)),
		eyewear("rimless-frames", "Rimless frames", (c) => (
			<LensPair context={c}>
				<rect
					x="-7"
					y="-5"
					width="14"
					height="11"
					rx="2"
					stroke={c.palette.sclera}
					strokeWidth="1"
				/>
				<path d="M-8 -1H-5M5 -1H8" stroke={c.palette.accent} strokeWidth="2" />
			</LensPair>
		)),
	],
	healthcare: [
		headwear("clinic-head-mirror", "Clinic head mirror", (p) => (
			<>
				<path d="M-23 4V-3Q0 -9 23 -3V4Q0 -3 -23 4Z" />
				<circle cx="0" cy="-7" r="12" fill={p.sclera} />
				<circle cx="0" cy="-7" r="8.5" stroke={p.accent} fill="none" />
				<circle cx="0" cy="-7" r="2.5" fill={p.ink} />
				<path d="M-7 -12L-4 -15" stroke={p.clothing} strokeWidth="2" />
			</>
		)),
		eyewear("protective-face-shield", "Protective face shield", (c) => {
			const { left, right, radius } = c.anchors.eyes;
			const x = left[0] - radius - 5;
			const y = Math.min(left[1], right[1]) - radius - 4;
			const width = right[0] - left[0] + radius * 2 + 10;
			return (
				<>
					<path
						d={`M${x} ${y}h${width}v${radius * 2 + 14}q-${width / 2} 9 -${width} 0Z`}
						fill={c.palette.sclera}
						fillOpacity=".16"
						stroke={c.palette.clothing}
					/>
					<path
						d={`M${x} ${y}h${width}`}
						stroke={c.palette.accent}
						strokeWidth="5"
					/>
					<path
						d={`M${x + 6} ${y + 5}v${radius * 2 + 3}M${x + 10} ${y + 6}v5`}
						stroke={c.palette.sclera}
						strokeWidth="2"
					/>
				</>
			);
		}),
		eyewear("laboratory-goggles", "Laboratory goggles", (c) => (
			<LensPair context={c}>
				<path
					d="M-8 -5Q0 -9 8 -5V4Q0 9 -8 4Z"
					stroke={c.palette.sclera}
					strokeWidth="3"
				/>
				<path
					d="M-6 -6V-9M0 -7V-10M6 -6V-9M-6 6V8M6 6V8"
					stroke={c.palette.accent}
					strokeWidth="1.5"
				/>
				<path d="M-5 -2L-2 -3" stroke={c.palette.clothing} />
			</LensPair>
		)),
	],
};

export const headwearBySeries: Readonly<
	Record<string, readonly BotEquipmentDefinition[]>
> = Object.fromEntries(
	Object.entries(everydayHeadwearBySeries).map(([series, items]) => [
		series,
		[...items, ...(specialtyHeadwearBySeries[series] ?? [])],
	]),
);
