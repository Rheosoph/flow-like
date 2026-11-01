import type { BotAnchors, BotPalette, BotShapeDefinition } from "../types";

type CrewShapeInput = Omit<BotShapeDefinition, "palette" | "anchors"> & {
	palette: Partial<BotPalette> & Pick<BotPalette, "body" | "background">;
	anchors?: Partial<BotAnchors>;
};

/** Shared attachment defaults; each character can override its proportions. */
export function crewShape(input: CrewShapeInput): BotShapeDefinition {
	return {
		eyeStyle: "dots",
		cheeks: false,
		...input,
		palette: {
			ink: "#302b35",
			sclera: "#fffdf6",
			cheek: "#eea494",
			clothing: "#456279",
			accent: "#f8c752",
			...input.palette,
		},
		anchors: {
			eyes: { left: [39, 65], right: [61, 65], radius: 3.8 },
			mouth: [50, 77],
			head: { x: 50, y: 35, width: 48 },
			neck: { y: 79, bottom: 100 },
			hand: [24, 84],
			...input.anchors,
		},
	};
}

export function ovalPath(cx: number, cy: number, rx: number, ry = rx): string {
	return `M${cx - rx} ${cy}a${rx} ${ry} 0 1 0 ${rx * 2} 0a${rx} ${ry} 0 1 0 ${-rx * 2} 0Z`;
}

export function radialPath(
	cx: number,
	cy: number,
	radii: readonly number[],
	repeats: number,
): string {
	const count = radii.length * repeats;
	return `M${Array.from({ length: count }, (_, index) => {
		const angle = (index / count) * Math.PI * 2 - Math.PI / 2;
		const radius = radii[index % radii.length];
		return `${(cx + Math.cos(angle) * radius).toFixed(2)} ${(cy + Math.sin(angle) * radius).toFixed(2)}`;
	}).join("L")}Z`;
}

export const ROUND_BODY = ovalPath(50, 71, 35, 32);
export const SQUIRCLE_BODY =
	"M17 64Q17 35 50 35Q83 35 83 64V84Q83 101 50 101Q17 101 17 84Z";
