import { describe, expect, it } from "bun:test";
import {
	contrastRatio,
	hexToOklch,
} from "../../lib/appearance/appearance-theme";
import { defaultBotCatalog } from "./catalog";
import {
	canUseBotColor,
	getBotColorSelectionPaint,
	normalizeBotHex,
} from "./colors";
import { randomizeBotColors } from "./random-colors";
import { canSaveBotAppearance } from "./save-eligibility";
import type {
	BotAppearance,
	BotCatalog,
	BotColorSlot,
	BotPaint,
	BotShapeDefinition,
} from "./types";

function seeded(seed: number) {
	let state = Math.imul(seed, 0x9e3779b9) >>> 0;
	return () => {
		state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
		return state / 0x100000000;
	};
}

function samples(paint: BotPaint): string[] {
	if (paint.kind === "solid") return [normalizeBotHex(paint.color) as string];
	if (paint.kind === "pattern") return [paint.base, paint.ink];
	const result = [...paint.colors];
	for (let index = 1; index < paint.colors.length; index++) {
		const a = Number.parseInt(paint.colors[index - 1].slice(1), 16);
		const b = Number.parseInt(paint.colors[index].slice(1), 16);
		for (const fraction of [0.25, 0.5, 0.75]) {
			const channels = [16, 8, 0].map((shift) => {
				const start = (a >> shift) & 255;
				const end = (b >> shift) & 255;
				return Math.round(start + (end - start) * fraction)
					.toString(16)
					.padStart(2, "0");
			});
			result.push(`#${channels.join("")}`);
		}
	}
	return result;
}

function paintFor(
	shape: BotShapeDefinition,
	colors: NonNullable<BotAppearance["colors"]>,
	slot: BotColorSlot,
	catalog = defaultBotCatalog,
): BotPaint {
	return (
		getBotColorSelectionPaint(colors[slot], catalog) ?? {
			kind: "solid",
			color: shape.palette[slot],
		}
	);
}

function minimumContrast(a: BotPaint, b: BotPaint): number {
	return Math.min(
		...samples(a).flatMap((first) =>
			samples(b).map((second) => contrastRatio(first, second)),
		),
	);
}

describe("bot random colors", () => {
	it("keeps exposed leaves, petals and other details visible against the background", () => {
		for (const shape of Object.values(defaultBotCatalog.shapes).filter(
			(shape) => shape.backgroundDetailColors?.length,
		)) {
			for (let seed = 1; seed <= 32; seed++) {
				const colors = randomizeBotColors({
					shape,
					catalog: defaultBotCatalog,
					random: seeded(seed),
				});
				for (const slot of shape.backgroundDetailColors ?? []) {
					expect(
						minimumContrast(
							paintFor(shape, colors, slot),
							paintFor(shape, colors, "background"),
						),
						`${shape.id}/${slot}/${seed}`,
					).toBeGreaterThanOrEqual(3);
				}
			}
		}
	});
	it.each(["mochi", "brief", "slate"])(
		"keeps the face, background and outfit distinct for %s",
		(kind) => {
			const shape = defaultBotCatalog.shapes[kind];
			for (let seed = 1; seed <= 32; seed++) {
				const colors = randomizeBotColors({
					shape,
					catalog: defaultBotCatalog,
					random: seeded(seed),
				});
				const paint = (slot: BotColorSlot) => paintFor(shape, colors, slot);
				expect(
					minimumContrast(paint("body"), paint("background")),
				).toBeGreaterThanOrEqual(3);
				if (shape.eyeStyle !== "light")
					expect(
						minimumContrast(paint("ink"), paint("sclera")),
					).toBeGreaterThanOrEqual(4.5);
				expect(
					minimumContrast(paint(shape.mouthColor ?? "ink"), paint("body")),
				).toBeGreaterThanOrEqual(3);
				expect(
					minimumContrast(paint("clothing"), paint("body")),
				).toBeGreaterThanOrEqual(2);
				if (shape.eyeStyle === "light") {
					expect(
						minimumContrast(paint("sclera"), paint("body")),
					).toBeGreaterThanOrEqual(3);
				} else if (shape.eyeStyle === "round" || shape.eyeStyle === "sleepy") {
					expect(
						minimumContrast(paint("sclera"), paint("body")),
					).toBeGreaterThanOrEqual(1.5);
				} else {
					expect(
						minimumContrast(paint("ink"), paint("body")),
					).toBeGreaterThanOrEqual(3);
				}
				for (const slot of ["ink", "sclera", "background", "accent"] as const)
					expect(paint(slot).kind).toBe("solid");
				expect(
					[paint("body"), paint("clothing")].filter(
						(value) => value.kind !== "solid",
					).length,
				).toBeLessThanOrEqual(1);
				expect(canSaveBotAppearance({ kind, colors }, defaultBotCatalog)).toBe(
					true,
				);
			}
		},
	);

	it("keeps key facial features and silhouettes distinct throughout the default crew", () => {
		for (const shape of Object.values(defaultBotCatalog.shapes)) {
			for (const seed of [1, 7, 19, 31]) {
				const colors = randomizeBotColors({
					shape,
					catalog: defaultBotCatalog,
					random: seeded(seed),
				});
				const paint = (slot: BotColorSlot) => paintFor(shape, colors, slot);
				expect(
					minimumContrast(paint("body"), paint("background")),
				).toBeGreaterThanOrEqual(3);
				if (shape.eyeStyle !== "light")
					expect(
						minimumContrast(paint("ink"), paint("sclera")),
					).toBeGreaterThanOrEqual(4.5);
				const eye = shape.eyeStyle === "light" ? "sclera" : "ink";
				const face = paint(shape.faceColor ?? "body");
				expect(minimumContrast(paint(eye), face)).toBeGreaterThanOrEqual(3);
				const mouth =
					shape.mouthStyle === "beak" ? "accent" : (shape.mouthColor ?? "ink");
				expect(minimumContrast(paint(mouth), face)).toBeGreaterThanOrEqual(3);
				expect(
					canSaveBotAppearance({ kind: shape.id, colors }, defaultBotCatalog),
				).toBe(true);
			}
		}
	});

	it.each([
		["shelly", "accent"],
		["vinyl", "accent"],
		["scope", "accent"],
		["sol", "accent"],
		["phial", "accent"],
		["astro", "clothing"],
		["tempo", "sclera"],
		["scout", "sclera"],
		["pixel", "sclera"],
		["toasty", "sclera"],
		["pearl", "sclera"],
		["badge", "sclera"],
	] as const)(
		"keeps %s features readable on its %s face panel",
		(kind, surface) => {
			const shape = defaultBotCatalog.shapes[kind];
			expect(shape.faceColor).toBe(surface);
			for (let seed = 1; seed <= 32; seed++) {
				const colors = randomizeBotColors({
					shape,
					catalog: defaultBotCatalog,
					random: seeded(seed),
				});
				const paint = (slot: BotColorSlot) => paintFor(shape, colors, slot);
				const eye = shape.eyeStyle === "light" ? "sclera" : "ink";
				const mouth =
					shape.mouthStyle === "beak" ? "accent" : (shape.mouthColor ?? "ink");
				expect(
					minimumContrast(paint(eye), paint(surface)),
				).toBeGreaterThanOrEqual(3);
				expect(
					minimumContrast(paint(mouth), paint(surface)),
				).toBeGreaterThanOrEqual(3);
				if (shape.eyeStyle !== "light")
					expect(
						minimumContrast(paint("ink"), paint("sclera")),
					).toBeGreaterThanOrEqual(4.5);
				if (
					surface !== "sclera" &&
					(!shape.eyeStyle ||
						shape.eyeStyle === "round" ||
						shape.eyeStyle === "sleepy")
				)
					expect(
						minimumContrast(paint("sclera"), paint(surface)),
					).toBeGreaterThanOrEqual(1.5);
				expect(canSaveBotAppearance({ kind, colors }, defaultBotCatalog)).toBe(
					true,
				);
			}
		},
	);

	it("still reaches gradient and patterned paints across varied seeds", () => {
		const shape = defaultBotCatalog.shapes.brief;
		const kinds = new Set<BotPaint["kind"]>();
		const bodies = new Set<string>();
		for (let seed = 1; seed <= 128; seed++) {
			const colors = randomizeBotColors({
				shape,
				catalog: defaultBotCatalog,
				random: seeded(seed),
			});
			for (const slot of ["body", "clothing"] as const)
				kinds.add(paintFor(shape, colors, slot).kind);
			bodies.add(JSON.stringify(colors.body));
		}
		expect(kinds).toEqual(new Set(["solid", "gradient", "pattern"]));
		expect(bodies.size).toBeGreaterThan(8);
	});

	it("usually pairs solid colors with muted backgrounds across light and dark looks", () => {
		const shape = defaultBotCatalog.shapes.brief;
		let light = 0;
		let dark = 0;
		let muted = 0;
		let solid = 0;
		const palettes = new Set<string>();
		for (let seed = 1; seed <= 128; seed++) {
			const colors = randomizeBotColors({
				shape,
				catalog: defaultBotCatalog,
				random: seeded(seed),
			});
			const background = paintFor(shape, colors, "background");
			expect(background.kind).toBe("solid");
			if (background.kind !== "solid") continue;
			const [lightness, chroma] = hexToOklch(background.color)
				.slice(6, -1)
				.split(" ")
				.map(Number);
			if (lightness >= 0.8) light++;
			if (lightness <= 0.5) dark++;
			if (chroma <= 0.08) muted++;
			if (
				(["body", "clothing"] as const).every(
					(slot) => paintFor(shape, colors, slot).kind === "solid",
				)
			)
				solid++;
			palettes.add(JSON.stringify(colors));
		}
		// Backgrounds should support the character without forcing every look onto charcoal.
		expect(muted).toBeGreaterThanOrEqual(116);
		expect(light).toBeGreaterThanOrEqual(26);
		expect(dark).toBeGreaterThanOrEqual(26);
		expect(solid).toBeGreaterThanOrEqual(84);
		expect(palettes.size).toBeGreaterThan(8);
	});

	it("uses a permitted custom color pack without depending on its ID or having swatches", () => {
		const shape = defaultBotCatalog.shapes.mochi;
		const catalog: BotCatalog = {
			...defaultBotCatalog,
			colors: {},
			colorPacks: {
				"studio-tints": {
					id: "studio-tints",
					label: "Studio tints",
					description: "Custom colors for the studio.",
					colorIds: [],
					access: "premium",
					allowsCustomHex: true,
				},
			},
		};
		for (const ownedPackIds of [undefined, ["basics-crew", "studio-tints"]]) {
			const colors = randomizeBotColors({
				shape,
				catalog,
				ownedPackIds,
				random: seeded(42),
			});
			expect(Object.keys(colors).length).toBeGreaterThan(0);
			for (const selection of Object.values(colors)) {
				expect(selection?.kind).toBe("hex");
				if (selection?.kind === "hex")
					expect(selection.packId).toBe("studio-tints");
				expect(canUseBotColor(selection, catalog, ownedPackIds)).toBe(true);
			}
			expect(
				canSaveBotAppearance({ kind: shape.id, colors }, catalog, ownedPackIds),
			).toBe(true);
		}
		expect(
			randomizeBotColors({
				shape,
				catalog,
				ownedPackIds: ["basics-crew"],
				random: seeded(42),
			}),
		).toEqual({});
	});

	it("falls back to available swatches when custom colors are unsupported or unowned", () => {
		const shape = defaultBotCatalog.shapes.mochi;
		const { "color-lab": _customPack, ...swatchPacks } =
			defaultBotCatalog.colorPacks;
		for (const { catalog, ownedPackIds } of [
			{
				catalog: { ...defaultBotCatalog, colorPacks: swatchPacks },
				ownedPackIds: undefined,
			},
			{ catalog: defaultBotCatalog, ownedPackIds: ["basics-crew"] },
		]) {
			for (let seed = 1; seed <= 16; seed++) {
				const colors = randomizeBotColors({
					shape,
					catalog,
					ownedPackIds,
					random: seeded(seed),
				});
				expect(Object.keys(colors).length).toBeGreaterThan(0);
				for (const selection of Object.values(colors)) {
					expect(selection?.kind).toBe("swatch");
					expect(canUseBotColor(selection, catalog, ownedPackIds)).toBe(true);
				}
				expect(
					canSaveBotAppearance(
						{ kind: shape.id, colors },
						catalog,
						ownedPackIds,
					),
				).toBe(true);
			}
		}
	});

	it("is deterministic for a seed and finishes with a bounded number of random choices", () => {
		const shape = defaultBotCatalog.shapes.brief;
		const options = { shape, catalog: defaultBotCatalog };
		expect(randomizeBotColors({ ...options, random: seeded(42) })).toEqual(
			randomizeBotColors({ ...options, random: seeded(42) }),
		);
		let calls = 0;
		const colors = randomizeBotColors({
			...options,
			random: () => {
				calls++;
				return 0.5;
			},
		});
		expect(calls).toBeGreaterThan(0);
		expect(calls).toBeLessThanOrEqual(10);
		expect(
			canSaveBotAppearance({ kind: shape.id, colors }, defaultBotCatalog),
		).toBe(true);
	});

	it("keeps ownership restrictions and fixed color slots", () => {
		const shape = {
			...defaultBotCatalog.shapes.mochi,
			colorSlots: ["body", "clothing"] as const,
		};
		const catalog = {
			...defaultBotCatalog,
			shapes: { ...defaultBotCatalog.shapes, [shape.id]: shape },
		};
		const ownedPackIds = ["basics-crew"];
		for (let seed = 1; seed <= 12; seed++) {
			const colors = randomizeBotColors({
				shape,
				catalog,
				ownedPackIds,
				random: seeded(seed),
			});
			expect(
				Object.keys(colors).every(
					(slot) => slot === "body" || slot === "clothing",
				),
			).toBe(true);
			for (const selection of Object.values(colors))
				expect(canUseBotColor(selection, catalog, ownedPackIds)).toBe(true);
			expect(
				canSaveBotAppearance({ kind: shape.id, colors }, catalog, ownedPackIds),
			).toBe(true);
		}
	});

	it("handles catalogs with no colors or no editable slots", () => {
		const shape = defaultBotCatalog.shapes.mochi;
		const empty = { ...defaultBotCatalog, colors: {}, colorPacks: {} };
		const {
			colors: _colors,
			colorPacks: _colorPacks,
			...legacy
		} = defaultBotCatalog;
		for (const catalog of [empty, legacy as BotCatalog])
			expect(randomizeBotColors({ shape, catalog, random: seeded(1) })).toEqual(
				{},
			);
		expect(
			randomizeBotColors({
				shape: { ...shape, colorSlots: [] },
				catalog: defaultBotCatalog,
				random: seeded(1),
			}),
		).toEqual({});
	});

	it("falls back without retries when a restricted palette cannot meet the targets", () => {
		const shape = {
			...defaultBotCatalog.shapes.mochi,
			palette: Object.fromEntries(
				Object.keys(defaultBotCatalog.shapes.mochi.palette).map((slot) => [
					slot,
					"#888888",
				]),
			) as unknown as BotShapeDefinition["palette"],
		};
		const catalog: BotCatalog = {
			...defaultBotCatalog,
			shapes: { ...defaultBotCatalog.shapes, [shape.id]: shape },
			colors: {
				gray: {
					id: "gray",
					label: "Gray",
					paint: { kind: "solid", color: "#888888" },
				},
			},
			colorPacks: {},
		};
		let calls = 0;
		const colors = randomizeBotColors({
			shape,
			catalog,
			random: () => {
				calls++;
				return 0;
			},
		});
		expect(calls).toBeLessThanOrEqual(10);
		expect(canSaveBotAppearance({ kind: shape.id, colors }, catalog)).toBe(
			true,
		);
	});

	it.each(["lcd", "blueprint", "thermal"] as const)(
		"does not generate hidden colors for the %s palette effect",
		(effect) => {
			expect(
				randomizeBotColors({
					shape: defaultBotCatalog.shapes.mochi,
					catalog: defaultBotCatalog,
					effect,
					random: seeded(1),
				}),
			).toEqual({});
		},
	);

	it("chooses the body against the fixed neon background", () => {
		const shape = defaultBotCatalog.shapes.brief;
		for (let seed = 1; seed <= 12; seed++) {
			const colors = randomizeBotColors({
				shape,
				catalog: defaultBotCatalog,
				effect: "neon",
				random: seeded(seed),
			});
			expect(colors.background).toBeUndefined();
			expect(
				minimumContrast(paintFor(shape, colors, "body"), {
					kind: "solid",
					color: "#100e1d",
				}),
			).toBeGreaterThanOrEqual(3);
		}
	});

	it.each(["gradient", "pattern"] as const)(
		"checks both light and dark parts of a %s before placing it behind the face",
		(kind) => {
			const shape = defaultBotCatalog.shapes.brief;
			const paint: BotPaint =
				kind === "gradient"
					? { kind, colors: ["#FFFFFF", "#000000"], animated: true }
					: { kind, base: "#FFFFFF", ink: "#000000", motif: "stripes" };
			const catalog: BotCatalog = {
				...defaultBotCatalog,
				colors: {
					...defaultBotCatalog.colors,
					ambiguous: { id: "ambiguous", label: "Mixed light and dark", paint },
				},
			};
			for (let seed = 1; seed <= 16; seed++) {
				const colors = randomizeBotColors({
					shape,
					catalog,
					random: seeded(seed),
				});
				expect(colors.body).not.toEqual({
					kind: "swatch",
					colorId: "ambiguous",
				});
				expect(
					minimumContrast(
						paintFor(shape, colors, "body", catalog),
						paintFor(shape, colors, "ink", catalog),
					),
				).toBeGreaterThanOrEqual(3);
			}
		},
	);
});
