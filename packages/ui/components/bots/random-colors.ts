import { hexToOklch, oklchToHex } from "../../lib/appearance/appearance-theme";
import { isBotPaletteEffect } from "./appearance";
import {
	canUseBotColor,
	getBotCustomizableColorSlots,
	normalizeBotHex,
} from "./colors";
import { BOT_RANDOM_PALETTES } from "./random-palettes";
import type {
	BotAppearance,
	BotCatalog,
	BotColorSelection,
	BotColorSlot,
	BotEffect,
	BotPaint,
	BotPalette,
	BotShapeDefinition,
} from "./types";

interface ColorChoice {
	selection?: BotColorSelection;
	complex: boolean;
	low: number;
	high: number;
	tones: number[][];
}

function channels(hex: string): number[] {
	const normalized = normalizeBotHex(hex) ?? "#000000";
	return [1, 3, 5].map(
		(start) => Number.parseInt(normalized.slice(start, start + 2), 16) / 255,
	);
}

// https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html
function luminance(rgb: readonly number[]): number {
	const linear = rgb.map((value) =>
		value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
	);
	return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
}

function choice(paint: BotPaint, selection?: BotColorSelection): ColorChoice {
	const colors =
		paint.kind === "solid"
			? [paint.color]
			: paint.kind === "pattern"
				? [paint.base, paint.ink]
				: paint.colors;
	const rgb = colors.map(channels);
	const values = rgb.map(luminance);
	if (paint.kind === "gradient") {
		// Component bounds also cover intermediate colors and animated stop transitions.
		values.push(
			luminance(
				[0, 1, 2].map((index) => Math.min(...rgb.map((c) => c[index]))),
			),
			luminance(
				[0, 1, 2].map((index) => Math.max(...rgb.map((c) => c[index]))),
			),
		);
	}
	return {
		selection,
		complex: paint.kind !== "solid",
		low: Math.min(...values),
		high: Math.max(...values),
		tones: colors.map((hex) =>
			hexToOklch(normalizeBotHex(hex) ?? "#000000")
				.slice(6, -1)
				.split(" ")
				.map(Number),
		),
	};
}

// Squared OKLab distance compares perceptual lightness and hue together.
function distance(color: ColorChoice, target: ColorChoice): number {
	const [lightness, chroma, hue] = target.tones[0];
	return (
		color.tones.reduce(
			(sum, [l, c, h]) =>
				sum +
				(l - lightness) ** 2 +
				c ** 2 +
				chroma ** 2 -
				2 * c * chroma * Math.cos(((h - hue) * Math.PI) / 180),
			0,
		) / color.tones.length
	);
}

function closest<T>(items: readonly T[], distance: (item: T) => number): T {
	return items.reduce((best, item) =>
		distance(item) < distance(best) ? item : best,
	);
}

function surfaceChoices<T>(
	items: readonly T[],
	color: (item: T) => ColorChoice,
	target: ColorChoice,
	textured: boolean,
): readonly T[] {
	if (textured) {
		const matching = items.filter(
			(item) => color(item).complex && distance(color(item), target) < 0.025,
		);
		if (matching.length) return matching;
	}
	const solids = items.filter((item) => !color(item).complex);
	return solids.length ? solids : items;
}

function contrast(a: ColorChoice, b: ColorChoice): number {
	if (a.low > b.high) return (a.low + 0.05) / (b.high + 0.05);
	if (b.low > a.high) return (b.low + 0.05) / (a.high + 0.05);
	return 1;
}

/** Keep all passing choices; use the strongest available contrast if a catalog is limited. */
function candidates<T>(items: readonly T[], score: (item: T) => number): T[] {
	const passing: T[] = [];
	let best = items[0];
	let bestScore = Number.NEGATIVE_INFINITY;
	for (const item of items) {
		const value = score(item);
		if (value >= 1) passing.push(item);
		if (value > bestScore) {
			best = item;
			bestScore = value;
		}
	}
	return passing.length ? passing : [best];
}

/** Select adjacent colors together, with a fixed number of passes over the available swatches. */
export function randomizeBotColors({
	shape,
	catalog,
	ownedPackIds,
	effect = "none",
	random = Math.random,
}: {
	shape: BotShapeDefinition;
	catalog: BotCatalog;
	ownedPackIds?: readonly string[];
	effect?: BotEffect;
	random?: () => number;
}): NonNullable<BotAppearance["colors"]> {
	const slots = getBotCustomizableColorSlots(shape);
	if (!slots.length || isBotPaletteEffect(effect)) return {};
	const pick = <T>(items: readonly T[]) =>
		items[
			Math.min(
				items.length - 1,
				Math.max(0, Math.floor(random() * items.length)),
			)
		];
	const theme = pick(BOT_RANDOM_PALETTES);
	// Inset faces need a neutral panel; a visor needs the theme's darker tone.
	const palette = {
		...theme,
		...(shape.faceColor === "accent" ? { accent: theme.sclera } : {}),
		...(shape.faceColor === "clothing" ? { clothing: theme.accent } : {}),
	};
	const customPack = Object.values(catalog.colorPacks ?? {}).find(
		(pack) =>
			pack.allowsCustomHex &&
			canUseBotColor(
				{ kind: "hex", value: palette.body, packId: pack.id },
				catalog,
				ownedPackIds,
			),
	);
	const targets = Object.fromEntries(
		Object.entries(palette).map(([slot, color]) => [
			slot,
			choice(
				{ kind: "solid", color },
				customPack
					? { kind: "hex", value: color, packId: customPack.id }
					: undefined,
			),
		]),
	) as Record<keyof BotPalette, ColorChoice>;
	const swatches = Object.values(catalog.colors ?? {}).flatMap((color) => {
		const selection = { kind: "swatch", colorId: color.id } as const;
		return canUseBotColor(selection, catalog, ownedPackIds)
			? [choice(color.paint, selection)]
			: [];
	});
	if (!swatches.length && !customPack) return {};
	const solids = swatches.filter((color) => !color.complex);
	const original = (slot: BotColorSlot) =>
		choice({ kind: "solid", color: shape.palette[slot] });
	const detailColors = new Set(shape.backgroundDetailColors ?? []);
	const detailTones = new Map<BotColorSlot, ColorChoice[]>();
	if (customPack) {
		for (const slot of detailColors) {
			const [, chroma, hue] = targets[slot].tones[0];
			detailTones.set(
				slot,
				[0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9].map((lightness) => {
					const color =
						normalizeBotHex(
							oklchToHex(`oklch(${lightness} ${chroma} ${hue})`),
						) ?? palette[slot];
					return choice(
						{ kind: "solid", color },
						{ kind: "hex", value: color, packId: customPack.id },
					);
				}),
			);
		}
	}
	const options = (slot: BotColorSlot, pool = swatches) =>
		slots.includes(slot)
			? [
					...(customPack ? [targets[slot]] : []),
					...(detailTones.get(slot) ?? []),
					original(slot),
					...pool,
				]
			: [original(slot)];
	const textureRoll = random();
	const textureSlot =
		textureRoll < 0.82
			? undefined
			: textureRoll < 0.94 && slots.includes("clothing")
				? "clothing"
				: "body";
	const backgrounds =
		effect === "neon"
			? [choice({ kind: "solid", color: "#100e1d" })]
			: options("background", solids);
	const tones = [...solids].sort((a, b) => a.low - b.low);
	const faceColors = [
		...(customPack ? [targets.ink, targets.sclera] : []),
		...(tones.length ? [tones[0], tones[tones.length - 1]] : []),
	];
	const inks = options("ink", faceColors);
	const scleras = options("sclera", faceColors);
	const faces = inks.flatMap((ink) =>
		scleras.map((sclera) => ({ ink, sclera })),
	);
	const faceDistance = (face: (typeof faces)[number]) =>
		distance(face.ink, targets.ink) + distance(face.sclera, targets.sclera);
	const faceRole = shape.faceColor ?? "body";
	const eyeRole = shape.eyeStyle === "light" ? "sclera" : "ink";
	const mouthRole = shape.mouthColor ?? "ink";
	const roundEyes =
		!shape.eyeStyle ||
		shape.eyeStyle === "round" ||
		shape.eyeStyle === "sleepy";
	const faceScore = (body: ColorChoice, face: (typeof faces)[number]) =>
		Math.min(
			// Light eyes are a single shape, without a pupil on an eye-white surface.
			shape.eyeStyle === "light" ? 1 : contrast(face.ink, face.sclera) / 4.5,
			contrast(body, face[eyeRole]) / 3,
			mouthRole === "ink" || mouthRole === "sclera"
				? contrast(body, face[mouthRole]) / 3
				: 1,
			roundEyes && faceRole !== "sclera"
				? contrast(body, face.sclera) / 1.5
				: 1,
		);
	const surfacePlan = (surface: ColorChoice) => {
		// An eye-white face panel and the eyes share one saved color slot.
		const surfaceFaces =
			faceRole === "sclera"
				? faces.map((face) => ({ ...face, sclera: surface }))
				: faces;
		const face = closest(
			candidates(surfaceFaces, (face) => faceScore(surface, face)),
			faceDistance,
		);
		return { surface, face, score: faceScore(surface, face) };
	};
	const faceSurfaces =
		faceRole === "body" ? [] : options(faceRole, solids).map(surfacePlan);
	const exposedOptions = [...detailColors].map((slot) => ({
		slot,
		colors: options(slot, solids),
	}));
	const bodies = options("body").map((body) => {
		const surfaces = (
			faceRole === "body" ? [surfacePlan(body)] : faceSurfaces
		).map((plan) => ({
			...plan,
			score: Math.min(
				plan.score,
				faceRole === "body"
					? 1
					: contrast(plan.surface, body) / (faceRole === "clothing" ? 2 : 1.5),
			),
		}));
		const { surface, face, score } = closest(
			candidates(surfaces, (plan) => plan.score),
			(plan) =>
				distance(plan.surface, targets[faceRole]) + faceDistance(plan.face),
		);
		const visibleColors: Partial<Record<BotColorSlot, ColorChoice>> = {
			body,
			ink: face.ink,
			sclera: face.sclera,
			[faceRole]: surface,
		};
		const backgroundScore = (color: ColorChoice) =>
			Math.min(
				contrast(body, color) / 3,
				...exposedOptions.map(({ slot, colors }) => {
					const selected = visibleColors[slot];
					if (selected) return contrast(selected, color) / 3;
					return Math.max(
						...colors.map((detail) =>
							Math.min(
								contrast(detail, color) / 3,
								contrast(detail, body) /
									(slot === "clothing" ? 2 : slot === "accent" ? 1.5 : 1),
							),
						),
					);
				}),
			);
		const background = closest(
			candidates(backgrounds, backgroundScore),
			(color) => distance(color, targets.background),
		);
		return {
			body,
			surface,
			face,
			background,
			score: Math.min(score, backgroundScore(background)),
		};
	});
	const plans = surfaceChoices(
		candidates(bodies, (plan) => plan.score),
		(plan) => plan.body,
		targets.body,
		textureSlot === "body",
	);
	const { body, surface, face, background } = closest(
		plans,
		(plan) =>
			distance(plan.body, targets.body) +
			distance(plan.surface, targets[faceRole]) +
			distance(plan.background, targets.background),
	);
	const outfits = candidates(
		options("clothing", body.complex ? solids : swatches),
		(color) =>
			Math.min(
				contrast(color, body) / 2,
				detailColors.has("clothing") ? contrast(color, background) / 3 : 1,
				mouthRole === "clothing" ? contrast(color, surface) / 3 : 1,
			),
	);
	const clothing =
		faceRole === "clothing"
			? surface
			: closest(
					surfaceChoices(
						candidates(outfits, (color) => contrast(color, face.ink) / 2.5),
						(color) => color,
						targets.clothing,
						textureSlot === "clothing" && !body.complex,
					),
					(color) => distance(color, targets.clothing),
				);
	const accents = candidates(options("accent", solids), (color) =>
		Math.min(
			contrast(color, body) / 1.5,
			detailColors.has("accent") ? contrast(color, background) / 3 : 1,
			shape.mouthStyle === "beak" || mouthRole === "accent"
				? contrast(color, surface) / 3
				: 1,
		),
	);
	const accent =
		faceRole === "accent"
			? surface
			: closest(
					candidates(accents, (color) => contrast(color, clothing) / 2),
					(color) => distance(color, targets.accent),
				);
	const cheek = closest(
		candidates(options("cheek", solids), (color) =>
			detailColors.has("cheek") ? contrast(color, background) / 3 : 1,
		),
		(color) => distance(color, targets.cheek),
	);
	const selected = { body, background, clothing, accent, cheek, ...face };
	const result: NonNullable<BotAppearance["colors"]> = {};
	for (const slot of Object.keys(selected) as (keyof typeof selected)[]) {
		const color = selected[slot];
		if (slots.includes(slot) && color.selection) result[slot] = color.selection;
	}
	return result;
}
