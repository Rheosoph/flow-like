import { describe, expect, it } from "bun:test";
import { createBotCatalog, defaultBotCatalog } from "./catalog";
import {
	canSaveBotAppearance,
	getBotSaveEligibility,
} from "./save-eligibility";
import {
	BOT_COLOR_SLOTS,
	type BotAppearance,
	type BotColorSelection,
} from "./types";

const swatch = (colorId: string): BotColorSelection => ({
	kind: "swatch",
	colorId,
});
const hex = (packId = "color-lab", value = "#abc"): BotColorSelection => ({
	kind: "hex",
	value,
	packId,
});
const catalog = createBotCatalog({
	packs: [
		{
			id: "favorites",
			label: "Favorites",
			description: "Favorite characters and accessories.",
			coverKind: "mochi",
			shapeIds: ["mochi"],
			equipmentIds: ["crown", "laptop"],
		},
		{
			id: "accessories",
			label: "Accessories",
			description: "Accessories for every character.",
			coverKind: "miso",
			shapeIds: ["miso"],
			equipmentIds: ["crown"],
		},
	],
	colorPacks: [
		{
			id: "mixed-colors",
			label: "Mixed colors",
			description: "A selection from other palettes.",
			access: "premium",
			colorIds: ["pastel-mint", "holo-aurora"],
		},
	],
});

describe("bot save eligibility", () => {
	it("checks replacement ownership and uniform ownership independently", () => {
		const appearance = { kind: "chatty", equipment: { neck: "doctor-coat" } };
		expect(
			canSaveBotAppearance(appearance, catalog, [
				"flow-crew",
				"healthcare-crew",
			]),
		).toBe(true);
		expect(
			getBotSaveEligibility(appearance, catalog, [
				"flow-crew",
			]).requirements.map((item) => item.id),
		).toEqual(["doctor-coat"]);
		expect(
			getBotSaveEligibility(appearance, catalog, [
				"healthcare-crew",
			]).requirements.map((item) => item.id),
		).toEqual(["bubble"]);
	});

	it("allows all catalog cosmetics when ownership restrictions are omitted", () => {
		const appearance: BotAppearance = {
			kind: "mochi",
			equipment: { head: "crown" },
		};
		expect(getBotSaveEligibility(appearance, catalog)).toEqual({
			canSave: true,
			requirements: [],
		});
		expect(
			getBotSaveEligibility(
				{ ...appearance, colors: { body: swatch("color-coral") } },
				catalog,
			).canSave,
		).toBe(true);
		for (const body of [swatch("pastel-mint"), swatch("holo-aurora"), hex()]) {
			expect(
				getBotSaveEligibility({ ...appearance, colors: { body } }, catalog),
			).toEqual({ canSave: true, requirements: [] });
		}
	});

	it("checks the default character when no shape is supplied", () => {
		expect(canSaveBotAppearance({}, defaultBotCatalog)).toBe(true);
		expect(
			getBotSaveEligibility({}, defaultBotCatalog, []).requirements,
		).toEqual([
			{
				kind: "shape",
				id: "mochi",
				label: defaultBotCatalog.shapes.mochi.label,
				packIds: ["basics-crew"],
			},
		]);
		expect(canSaveBotAppearance({}, defaultBotCatalog, ["basics-crew"])).toBe(
			true,
		);
	});

	it("reports alternative packs per item and accepts any owned alternative", () => {
		const appearance: BotAppearance = {
			kind: "mochi",
			equipment: { head: "crown" },
			colors: { body: swatch("pastel-mint"), accent: swatch("holo-aurora") },
		};
		const result = getBotSaveEligibility(appearance, catalog, []);
		expect(
			result.requirements.map(({ kind, id, packIds }) => ({
				kind,
				id,
				packIds,
			})),
		).toEqual([
			{ kind: "shape", id: "mochi", packIds: ["basics-crew", "favorites"] },
			{ kind: "equipment", id: "crown", packIds: ["favorites", "accessories"] },
			{
				kind: "color",
				id: "pastel-mint",
				packIds: ["colors-pastels", "mixed-colors"],
			},
			{
				kind: "color",
				id: "holo-aurora",
				packIds: ["colors-holographic", "mixed-colors"],
			},
		]);
		for (const owned of [
			["favorites", "mixed-colors"],
			["basics-crew", "accessories", "colors-pastels", "colors-holographic"],
		])
			expect(canSaveBotAppearance(appearance, catalog, owned)).toBe(true);
		expect(
			getBotSaveEligibility(appearance, catalog, [
				"favorites",
			]).requirements.map((item) => item.kind),
		).toEqual(["color", "color"]);
	});

	it("checks all seven persisted color roles even when their paints are hidden", () => {
		const colors = Object.fromEntries(
			BOT_COLOR_SLOTS.map((slot) => [slot, swatch("holo-aurora")]),
		) as BotAppearance["colors"];
		for (const effect of ["none", "lcd", "blueprint"] as const) {
			const appearance: BotAppearance = {
				kind: "mochi",
				effect,
				dark: true,
				colors,
			};
			const result = getBotSaveEligibility(appearance, catalog, [
				"basics-crew",
			]);
			expect(result.requirements.map((item) => item.slot)).toEqual([
				...BOT_COLOR_SLOTS,
			]);
			expect(
				canSaveBotAppearance(appearance, catalog, [
					"basics-crew",
					"colors-holographic",
				]),
			).toBe(true);
		}
	});

	it("accepts colors shared with an included pack without demanding a premium pack", () => {
		const included = createBotCatalog(
			{
				colorPacks: [
					{
						id: "included-mint",
						label: "Included mint",
						description: "Included shared color.",
						access: "included",
						colorIds: ["pastel-mint"],
					},
				],
			},
			catalog,
		);
		expect(
			canSaveBotAppearance(
				{ kind: "mochi", colors: { body: swatch("pastel-mint") } },
				included,
				["basics-crew"],
			),
		).toBe(true);
	});

	it("requires the selected hex grant and accepts deliberate included hex grants", () => {
		const extended = createBotCatalog({
			colorPacks: [
				{
					id: "another-lab",
					label: "Another lab",
					description: "Custom colors.",
					access: "premium",
					allowsCustomHex: true,
					colorIds: [],
				},
				{
					id: "included-lab",
					label: "Included lab",
					description: "Included custom colors.",
					access: "included",
					allowsCustomHex: true,
					colorIds: [],
				},
			],
		});
		const appearance: BotAppearance = {
			kind: "mochi",
			colors: { body: hex() },
		};
		for (const owned of [["basics-crew"], ["basics-crew", "another-lab"]]) {
			expect(
				getBotSaveEligibility(appearance, extended, owned).requirements,
			).toEqual([
				{
					kind: "color",
					id: "#AABBCC",
					label: "Custom #AABBCC",
					slot: "body",
					packIds: ["color-lab"],
				},
			]);
		}
		expect(
			canSaveBotAppearance(appearance, extended, ["basics-crew", "color-lab"]),
		).toBe(true);
		expect(
			canSaveBotAppearance(
				{ ...appearance, colors: { body: hex("included-lab") } },
				extended,
			),
		).toBe(true);
	});

	it("keeps unbundled character and equipment usable but blocks unbundled colors", () => {
		const extended = createBotCatalog({
			shapes: [{ ...catalog.shapes.mochi, id: "free-shape" }],
			equipment: [{ ...catalog.equipment.crown, id: "free-hat" }],
			colors: [
				{
					id: "loose-color",
					label: "Loose color",
					paint: { kind: "solid", color: "#abc" },
				},
			],
		});
		const appearance: BotAppearance = {
			kind: "free-shape",
			equipment: { head: "free-hat" },
		};
		expect(canSaveBotAppearance(appearance, extended, [])).toBe(true);
		expect(
			getBotSaveEligibility(
				{ ...appearance, colors: { body: swatch("loose-color") } },
				extended,
				[],
			).requirements,
		).toEqual([
			{
				kind: "color",
				id: "loose-color",
				label: "Loose color",
				slot: "body",
				packIds: [],
			},
		]);
	});

	it("rejects invalid requested shapes instead of saving their rendered fallback", () => {
		for (const kind of [
			"missing",
			"constructor",
			"__proto__",
			"",
			null,
			4,
			{},
		]) {
			const input = { kind } as BotAppearance;
			const result = getBotSaveEligibility(input, catalog, [
				"flow-crew",
				"basics-crew",
			]);
			expect(result.canSave).toBe(false);
			expect(result.requirements).toHaveLength(1);
			expect(result.requirements[0]).toMatchObject({
				kind: "shape",
				label: "Unavailable character",
				packIds: [],
			});
		}
	});

	it("rejects unknown, incompatible and misplaced equipment despite ownership", () => {
		const extended = createBotCatalog(
			{
				equipment: [
					{ ...catalog.equipment.crown, id: "miso-hat", kinds: ["miso"] },
				],
			},
			catalog,
		);
		for (const equipment of [
			{ head: "missing" },
			{ head: "constructor" },
			{ head: "miso-hat" },
			{ hand: "crown" },
			{ feet: "crown" },
			{ head: null },
			{ head: {} },
		]) {
			const result = getBotSaveEligibility(
				{ kind: "mochi", equipment } as BotAppearance,
				extended,
				["favorites"],
			);
			expect(result.canSave).toBe(false);
			expect(result.requirements).toHaveLength(1);
			expect(result.requirements[0]).toMatchObject({
				kind: "equipment",
				packIds: [],
			});
		}
	});

	it("rejects excluded equipment even when its pack is owned", () => {
		const extended = createBotCatalog(
			{
				equipment: [
					{
						...catalog.equipment.crown,
						kinds: ["mochi", "miso"],
						excludedKinds: ["mochi"],
					},
				],
			},
			catalog,
		);
		expect(
			getBotSaveEligibility(
				{ kind: "mochi", equipment: { head: "crown" } },
				extended,
				["favorites"],
			),
		).toEqual({
			canSave: false,
			requirements: [
				{
					kind: "equipment",
					id: "crown",
					label: catalog.equipment.crown.label,
					slot: "head",
					packIds: [],
				},
			],
		});
		expect(
			canSaveBotAppearance(
				{ kind: "miso", equipment: { head: "crown" } },
				extended,
			),
		).toBe(true);
	});

	it("rejects malformed and unsupported colors instead of dropping them during saving", () => {
		const limited = createBotCatalog(
			{ shapes: [{ ...catalog.shapes.mochi, colorSlots: ["body"] }] },
			catalog,
		);
		for (const colors of [
			{ body: swatch("missing") },
			{ body: swatch("constructor") },
			{ body: hex("missing") },
			{ body: hex("colors-pastels") },
			{ body: hex("color-lab", "url(#paint)") },
			{ body: { kind: "bogus" } },
			{ body: null },
			{ body: "#ffffff" },
			{ accent: swatch("color-coral") },
			{ unknownSlot: swatch("color-coral") },
		]) {
			const result = getBotSaveEligibility(
				{ kind: "mochi", colors } as BotAppearance,
				limited,
				["favorites", "color-lab", "colors-pastels"],
			);
			expect(result.canSave).toBe(false);
			expect(result.requirements).toHaveLength(1);
			expect(result.requirements[0]).toMatchObject({
				kind: "color",
				packIds: [],
			});
		}
		expect(
			canSaveBotAppearance(
				{
					kind: "mochi",
					colors: { body: undefined, accent: undefined },
					equipment: { head: undefined },
				},
				limited,
				["favorites"],
			),
		).toBe(true);
	});

	it("rejects malformed containers without throwing or echoing their contents into labels", () => {
		for (const value of [null, false, "<script>bad</script>", 4, []]) {
			for (const key of ["equipment", "colors"]) {
				const result = getBotSaveEligibility(
					{ kind: "mochi", [key]: value } as BotAppearance,
					catalog,
				);
				expect(result.canSave).toBe(false);
				expect(result.requirements[0].packIds).toEqual([]);
				expect(result.requirements[0].label).not.toContain("<script>");
			}
			expect(
				canSaveBotAppearance(value as unknown as BotAppearance, catalog),
			).toBe(false);
		}
	});

	it("supports registered prototype-like IDs through own catalog entries", () => {
		const extended = createBotCatalog({
			shapes: [{ ...catalog.shapes.mochi, id: "constructor" }],
			equipment: [{ ...catalog.equipment.crown, id: "toString" }],
			colors: [
				{
					id: "__proto__",
					label: "Prototype color",
					paint: { kind: "solid", color: "#abc" },
				},
			],
			colorPacks: [
				{
					id: "prototype-colors",
					label: "Prototype colors",
					description: "Registered IDs.",
					access: "premium",
					colorIds: ["__proto__"],
				},
			],
		});
		const appearance: BotAppearance = {
			kind: "constructor",
			equipment: { head: "toString" },
			colors: { body: swatch("__proto__") },
		};
		expect(
			canSaveBotAppearance(appearance, extended, ["prototype-colors"]),
		).toBe(true);
		expect(
			getBotSaveEligibility(appearance, extended, []).requirements[0].packIds,
		).toEqual(["prototype-colors"]);
	});

	it("rechecks ownership after revocation without mutating the appearance or catalog", () => {
		const appearance: BotAppearance = Object.freeze({
			kind: "mochi",
			equipment: Object.freeze({ head: "crown" }),
			colors: Object.freeze({ body: Object.freeze(swatch("pastel-mint")) }),
		});
		const before = JSON.stringify(appearance);
		expect(
			canSaveBotAppearance(appearance, catalog, ["favorites", "mixed-colors"]),
		).toBe(true);
		const result = getBotSaveEligibility(appearance, catalog, []);
		expect(result.canSave).toBe(false);
		expect(result.requirements).toHaveLength(3);
		(result.requirements[0].packIds as string[]).push("fake");
		expect(
			getBotSaveEligibility(appearance, catalog, []).requirements[0].packIds,
		).not.toContain("fake");
		expect(JSON.stringify(appearance)).toBe(before);
	});
});
