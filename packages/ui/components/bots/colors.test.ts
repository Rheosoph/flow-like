import { describe, expect, it } from "bun:test";
import { getBotPalette, resolveBotAppearance } from "./appearance";
import { createBotCatalog, defaultBotCatalog } from "./catalog";
import {
	canUseBotColor,
	getBotColorBase,
	getBotColorPacks,
	getBotColorSelectionPaint,
	getBotCustomizableColorSlots,
	normalizeBotHex,
} from "./colors";
import {
	BOT_COLOR_SLOTS,
	type BotAppearance,
	type BotCatalog,
	type BotColorDefinition,
	type BotColorPackDefinition,
	type BotColorSelection,
	type BotColorSlot,
	type BotPaint,
} from "./types";

const swatch = (colorId: string): BotColorSelection => ({
	kind: "swatch",
	colorId,
});
const hex = (value = "#abc", packId = "color-lab"): BotColorSelection => ({
	kind: "hex",
	value,
	packId,
});
const customColor = (): BotColorDefinition => ({
	id: "custom-color",
	label: "Custom",
	paint: { kind: "solid", color: "#abc" },
});
const customPack = (): BotColorPackDefinition => ({
	id: "custom-colors",
	label: "Custom",
	description: "Custom palette.",
	colorIds: ["custom-color"],
	access: "premium",
});

describe("bot color catalog", () => {
	it("provides included essentials, premium palettes, every pattern and custom hex", () => {
		expect(Object.keys(defaultBotCatalog.colorPacks)).toEqual([
			"colors-essentials",
			"colors-pastels",
			"colors-jewels",
			"colors-holographic",
			"colors-patterns",
			"color-lab",
		]);
		const essentials = defaultBotCatalog.colorPacks["colors-essentials"];
		expect(essentials.access).toBe("included");
		expect(essentials.colorIds).toHaveLength(8);
		for (const id of essentials.colorIds) {
			expect(defaultBotCatalog.colors[id].paint.kind).toBe("solid");
			expect(canUseBotColor(swatch(id), defaultBotCatalog)).toBe(true);
		}
		const holo = defaultBotCatalog.colorPacks["colors-holographic"];
		expect(holo.colorIds).toHaveLength(4);
		for (const id of holo.colorIds)
			expect(defaultBotCatalog.colors[id].paint).toMatchObject({
				kind: "gradient",
				animated: true,
			});
		expect(
			defaultBotCatalog.colorPacks["colors-patterns"].colorIds.map((id) => {
				const paint = defaultBotCatalog.colors[id].paint;
				return paint.kind === "pattern" ? paint.motif : undefined;
			}),
		).toEqual(["dots", "stripes", "stars", "diamonds", "hearts"]);
		expect(defaultBotCatalog.colorPacks["color-lab"]).toMatchObject({
			access: "premium",
			colorIds: [],
			allowsCustomHex: true,
		});
		for (const colorId of Object.keys(defaultBotCatalog.colors))
			expect(getBotColorPacks(colorId, defaultBotCatalog)).toHaveLength(1);
	});

	it("normalizes three- and six-digit hex while rejecting CSS and other formats", () => {
		expect(normalizeBotHex(" #aBc ")).toBe("#AABBCC");
		expect(normalizeBotHex("#12efAB")).toBe("#12EFAB");
		for (const value of [
			null,
			undefined,
			0xff0000,
			{},
			[],
			"",
			"red",
			"transparent",
			"#12",
			"#1234",
			"#12345678",
			"#ggg",
			"rgb(1, 2, 3)",
			"url(https://example.com/a.svg)",
			"var(--color)",
			"#fff;fill:red",
		]) {
			expect(normalizeBotHex(value)).toBeUndefined();
		}
	});

	it("keeps premium swatches and hex locked without matching ownership", () => {
		for (const owned of [undefined, [], ["basics-crew"], ["colors-pastels"]]) {
			expect(
				canUseBotColor(swatch("holo-aurora"), defaultBotCatalog, owned),
			).toBe(false);
			expect(canUseBotColor(hex(), defaultBotCatalog, owned)).toBe(false);
		}
		expect(
			canUseBotColor(swatch("holo-aurora"), defaultBotCatalog, [
				"colors-holographic",
			]),
		).toBe(true);
		expect(canUseBotColor(hex(), defaultBotCatalog, ["color-lab"])).toBe(true);
		expect(canUseBotColor(swatch("color-sky"), defaultBotCatalog, [])).toBe(
			true,
		);
	});

	it("unlocks overlapping colors through any included or owned containing pack", () => {
		const first = createBotCatalog({
			colors: [customColor()],
			colorPacks: [customPack(), { ...customPack(), id: "another-colors" }],
		});
		expect(
			getBotColorPacks("custom-color", first).map((pack) => pack.id),
		).toEqual(["custom-colors", "another-colors"]);
		expect(canUseBotColor(swatch("custom-color"), first)).toBe(false);
		expect(
			canUseBotColor(swatch("custom-color"), first, ["another-colors"]),
		).toBe(true);
		const second = createBotCatalog(
			{ colorPacks: [{ ...customPack(), access: "included" }] },
			first,
		);
		expect(canUseBotColor(swatch("custom-color"), second)).toBe(true);
		expect(first.colorPacks["custom-colors"].access).toBe("premium");
	});

	it("resolves locked and unbundled paints for saved artwork without granting save access", () => {
		const catalog = createBotCatalog({ colors: [customColor()] });
		expect(getBotColorSelectionPaint(swatch("custom-color"), catalog)).toEqual({
			kind: "solid",
			color: "#AABBCC",
		});
		expect(canUseBotColor(swatch("custom-color"), catalog, [])).toBe(false);
		expect(
			getBotColorSelectionPaint(swatch("holo-aurora"), catalog)?.kind,
		).toBe("gradient");
		expect(getBotColorSelectionPaint(hex(), catalog)).toEqual({
			kind: "solid",
			color: "#AABBCC",
		});
	});

	it("rejects unknown, inherited and unauthorized hex-pack references", () => {
		for (const id of ["missing", "toString", "constructor", "__proto__"]) {
			expect(getBotColorPacks(id, defaultBotCatalog)).toEqual([]);
			expect(
				getBotColorSelectionPaint(swatch(id), defaultBotCatalog),
			).toBeUndefined();
			expect(
				getBotColorSelectionPaint(hex("#fff", id), defaultBotCatalog),
			).toBeUndefined();
			expect(canUseBotColor(swatch(id), defaultBotCatalog, [id])).toBe(false);
		}
		expect(
			getBotColorSelectionPaint(
				hex("#fff", "colors-essentials"),
				defaultBotCatalog,
			),
		).toBeUndefined();
		expect(
			canUseBotColor(hex("url(#paint)"), defaultBotCatalog, ["color-lab"]),
		).toBe(false);
	});

	it("handles minimal pre-color catalogs without throwing", () => {
		const {
			colors: _colors,
			colorPacks: _packs,
			...legacy
		} = defaultBotCatalog;
		const catalog = legacy as BotCatalog;
		expect(getBotColorPacks("color-sky", catalog)).toEqual([]);
		expect(getBotColorSelectionPaint(hex(), catalog)).toBeUndefined();
		expect(canUseBotColor(swatch("color-sky"), catalog)).toBe(false);
		expect(
			resolveBotAppearance({ colors: { body: swatch("color-sky") } }, catalog)
				.colors,
		).toEqual({});
		const extended = createBotCatalog(
			{ colors: [customColor()], colorPacks: [customPack()] },
			catalog,
		);
		expect(extended.colors["custom-color"].paint).toEqual({
			kind: "solid",
			color: "#AABBCC",
		});
	});

	it("treats registered prototype-like color and pack IDs as own entries", () => {
		const catalog = createBotCatalog({
			colors: [{ ...customColor(), id: "constructor" }],
			colorPacks: [
				{
					...customPack(),
					id: "__proto__",
					colorIds: ["constructor"],
					allowsCustomHex: true,
				},
			],
		});
		expect(Object.hasOwn(catalog.colors, "constructor")).toBe(true);
		expect(canUseBotColor(swatch("constructor"), catalog, ["__proto__"])).toBe(
			true,
		);
		expect(
			canUseBotColor(hex("#fff", "__proto__"), catalog, ["__proto__"]),
		).toBe(true);
		expect(canUseBotColor(swatch("constructor"), catalog)).toBe(false);
	});

	it("copies and freezes paints, stop arrays, pack membership and editable slots", () => {
		const colors = ["#abc", "#def"];
		const colorIds = ["custom-color"];
		const colorSlots: BotColorSlot[] = ["body", "accent"];
		const color: BotColorDefinition = {
			...customColor(),
			paint: { kind: "gradient", colors, angle: 30, animated: true },
		};
		const pack = { ...customPack(), colorIds };
		const shape = {
			...defaultBotCatalog.shapes.mochi,
			id: "custom-shape",
			colorSlots,
		};
		const catalog = createBotCatalog({
			colors: [color],
			colorPacks: [pack],
			shapes: [shape],
		});
		colors[0] = "#000";
		colorIds.push("color-sky");
		colorSlots.push("ink");
		color.label = "Changed";
		expect(catalog.colors["custom-color"].paint).toEqual({
			kind: "gradient",
			colors: ["#AABBCC", "#DDEEFF"],
			angle: 30,
			animated: true,
		});
		expect(catalog.colors["custom-color"].label).toBe("Custom");
		expect(catalog.colorPacks["custom-colors"].colorIds).toEqual([
			"custom-color",
		]);
		expect(
			getBotCustomizableColorSlots(catalog.shapes["custom-shape"]),
		).toEqual(["body", "accent"]);
		const paint = catalog.colors["custom-color"].paint;
		for (const value of [
			catalog.colors,
			catalog.colorPacks,
			catalog.colors["custom-color"],
			paint,
			paint.kind === "gradient" ? paint.colors : null,
			catalog.colorPacks["custom-colors"],
			catalog.colorPacks["custom-colors"].colorIds,
			catalog.shapes["custom-shape"].colorSlots,
		])
			expect(Object.isFrozen(value)).toBe(true);
		for (const value of [
			color,
			color.paint,
			colors,
			pack,
			colorIds,
			colorSlots,
		])
			expect(Object.isFrozen(value)).toBe(false);
	});

	it("returns solid representative colors and defaults to all seven editable roles", () => {
		expect(getBotColorBase({ kind: "solid", color: "#abc" })).toBe("#AABBCC");
		expect(
			getBotColorBase({ kind: "gradient", colors: ["#def", "#fff"] }),
		).toBe("#DDEEFF");
		expect(
			getBotColorBase({
				kind: "pattern",
				base: "#123",
				ink: "#fff",
				motif: "stars",
			}),
		).toBe("#112233");
		expect(
			getBotCustomizableColorSlots(defaultBotCatalog.shapes.mochi),
		).toEqual(BOT_COLOR_SLOTS);
		expect(
			getBotCustomizableColorSlots({
				...defaultBotCatalog.shapes.mochi,
				colorSlots: [],
			}),
		).toEqual([]);
	});

	it("sanitizes saved selections and omits roles the shape cannot customize", () => {
		const catalog = createBotCatalog({
			shapes: [
				{
					...defaultBotCatalog.shapes.mochi,
					id: "limited",
					colorSlots: ["body", "accent", "ink", "clothing"],
				},
			],
		});
		const input = {
			kind: "limited",
			colors: {
				body: { ...swatch("holo-aurora"), unexpected: true },
				accent: hex("#aBc"),
				background: swatch("color-sky"),
				ink: { kind: "raw", value: "url(#unsafe)" },
				clothing: hex("#fff", "missing"),
				notARole: swatch("color-sky"),
			},
		} as unknown as BotAppearance;
		expect(resolveBotAppearance(input, catalog).colors).toEqual({
			body: swatch("holo-aurora"),
			accent: hex("#AABBCC"),
		});
		for (const selection of [
			null,
			{},
			[],
			"red",
			{ kind: "swatch", colorId: 42 },
			{ kind: "hex", value: "#fff" },
		]) {
			expect(
				resolveBotAppearance(
					{ colors: { body: selection } } as BotAppearance,
					catalog,
				).colors,
			).toEqual({});
		}
	});

	it("applies custom solids before dark background and palette effects", () => {
		const appearance = resolveBotAppearance(
			{
				kind: "mochi",
				colors: {
					body: swatch("color-sky"),
					background: swatch("pattern-stars"),
					accent: hex(),
				},
			},
			defaultBotCatalog,
		);
		const palette = defaultBotCatalog.shapes.mochi.palette;
		const result = getBotPalette(palette, appearance);
		expect(result.body).toBe("#72BCE4");
		expect(result.background).toBe("#655C9E");
		expect(result.accent).toBe("#AABBCC");
		expect(result.ink).toBe(palette.ink);
		expect(palette.body).not.toBe(result.body);
		expect(
			getBotPalette(palette, { ...appearance, dark: true }).background,
		).toBe("#232335");
		for (const effect of ["lcd", "blueprint", "thermal"] as const) {
			expect(
				getBotPalette(palette, { ...appearance, dark: true, effect }),
			).toEqual(getBotPalette(palette, { ...appearance, colors: {}, effect }));
		}
		const catalog = createBotCatalog({ colors: [customColor()] });
		expect(
			getBotPalette(
				palette,
				resolveBotAppearance(
					{ colors: { body: swatch("custom-color") } },
					catalog,
				),
				catalog,
			).body,
		).toBe("#AABBCC");
	});

	it("keeps LCD to four tones and gives night effects their own background", () => {
		const base = defaultBotCatalog.shapes.mochi.palette;
		const appearance = resolveBotAppearance(
			{
				kind: "mochi",
				colors: { body: swatch("color-sky") },
			},
			defaultBotCatalog,
		);
		expect(
			new Set(
				Object.values(getBotPalette(base, { ...appearance, effect: "lcd" })),
			).size,
		).toBe(4);
		const neon = getBotPalette(base, {
			...appearance,
			effect: "neon",
			dark: true,
		});
		expect(neon.background).toBe("#100e1d");
		expect(neon.body).toBe("#72BCE4");
		const thermal = getBotPalette(base, { ...appearance, effect: "thermal" });
		expect(thermal.background).toBe("#17122d");
		expect(thermal.body).toBe("#ff9200");
		expect(getBotPalette(base, appearance).body).toBe("#72BCE4");
	});

	it("rejects malformed paints and CSS values in every color position", () => {
		const invalidPaints: unknown[] = [
			null,
			{},
			{ kind: "image", url: "https://example.com" },
			{ kind: "solid", color: "red" },
			{ kind: "solid", color: "#12345678" },
			{ kind: "gradient", colors: [] },
			{ kind: "gradient", colors: ["#fff"] },
			{ kind: "gradient", colors: ["#fff", "url(#paint)"] },
			{ kind: "gradient", colors: ["#fff", "#000"], angle: Number.NaN },
			{ kind: "gradient", colors: ["#fff", "#000"], angle: "90" },
			{ kind: "gradient", colors: ["#fff", "#000"], animated: "yes" },
			{ kind: "pattern", base: "#fff", ink: "#000", motif: "unknown" },
			{ kind: "pattern", base: "var(--color)", ink: "#000", motif: "dots" },
			{ kind: "pattern", base: "#fff", ink: "red", motif: "dots" },
			...[0, -1, Number.POSITIVE_INFINITY, "10"].map((scale) => ({
				kind: "pattern",
				base: "#fff",
				ink: "#000",
				motif: "dots",
				scale,
			})),
		];
		for (const paint of invalidPaints)
			expect(() =>
				createBotCatalog({
					colors: [{ ...customColor(), paint: paint as BotPaint }],
				}),
			).toThrow();
	});

	it("rejects empty packs unless hex is enabled and rejects invalid membership and access", () => {
		const invalidPacks: unknown[] = [
			{ ...customPack(), colorIds: [] },
			{ ...customPack(), colorIds: [], allowsCustomHex: false },
			{ ...customPack(), colorIds: [], allowsCustomHex: "yes" },
			{ ...customPack(), colorIds: undefined, allowsCustomHex: true },
			{ ...customPack(), colorIds: ["missing"] },
			{ ...customPack(), colorIds: ["toString"] },
			{ ...customPack(), colorIds: [" "] },
			{ ...customPack(), colorIds: ["custom-color", "custom-color"] },
			{ ...customPack(), access: "free" },
		];
		for (const pack of invalidPacks)
			expect(() =>
				createBotCatalog({
					colors: [customColor()],
					colorPacks: [pack as BotColorPackDefinition],
				}),
			).toThrow();
		const catalog = createBotCatalog({
			colorPacks: [
				{
					...customPack(),
					colorIds: [],
					access: "included",
					allowsCustomHex: true,
				},
			],
		});
		expect(canUseBotColor(hex("#fff", "custom-colors"), catalog)).toBe(true);
	});

	it("rejects duplicate IDs, invalid role metadata and character/color pack collisions", () => {
		expect(() =>
			createBotCatalog({ colors: [customColor(), customColor()] }),
		).toThrow("Duplicate bot color");
		expect(() =>
			createBotCatalog({ colors: [{ ...customColor(), id: " " }] }),
		).toThrow("nonempty id");
		expect(() =>
			createBotCatalog({
				colors: [customColor()],
				colorPacks: [customPack(), customPack()],
			}),
		).toThrow("Duplicate bot color pack");
		expect(() =>
			createBotCatalog({
				colorPacks: [
					{
						...customPack(),
						id: "basics-crew",
						colorIds: [],
						allowsCustomHex: true,
					},
				],
			}),
		).toThrow("shares an ID");
		expect(() =>
			createBotCatalog({
				packs: [{ ...defaultBotCatalog.packs["basics-crew"], id: "color-lab" }],
			}),
		).toThrow("shares an ID");
		for (const colorSlots of [["body", "body"], ["unknown"], "body"]) {
			expect(() =>
				createBotCatalog({
					shapes: [
						{
							...defaultBotCatalog.shapes.mochi,
							colorSlots: colorSlots as BotColorSlot[],
						},
					],
				}),
			).toThrow("color slot");
		}
	});
});
