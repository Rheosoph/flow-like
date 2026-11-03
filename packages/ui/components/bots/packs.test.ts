import { describe, expect, it } from "bun:test";
import { createBotCatalog, defaultBotCatalog } from "./catalog";
import { clothingBySeries } from "./equipment/clothing";
import { handPropsBySeries } from "./equipment/hand-props";
import { headwearBySeries } from "./equipment/headwear";
import {
	canUseBotEquipment,
	canUseBotShape,
	getBotEquipmentPacks,
	getBotPackContents,
	getBotShapePacks,
} from "./packs";
import type { BotPackDefinition } from "./types";

const EXPECTED_PACKS = {
	"flow-crew": ["flux", "lava", "magma", "bubble"],
	"basics-crew": ["pip", "mochi", "hexa", "pebble", "nimbus"],
	"nature-crew": ["sprout", "drip", "ember", "porci", "bloom"],
	"pets-crew": ["miso", "bun", "kuma", "fynn", "hami"],
	"zoo-crew": ["kero", "axi", "pino", "piyo", "bambu"],
	"tech-crew": ["nodi", "bit", "chip", "cog", "floppy"],
	"snacks-crew": ["momo", "nori", "toasty", "cupi", "puri"],
	"cosmos-crew": ["orbi", "sol", "stella", "roki", "astro"],
	"pro-crew": ["atlas", "prism", "slate", "facet", "otto"],
	"office-crew": ["clip", "jot", "memo", "rubi", "ruler", "binder", "klick"],
	"workshop-crew": ["wrench", "mallet", "bolt", "brush", "spool", "measure"],
	"ocean-crew": ["inky", "shelly", "bubbles", "cora", "jelly", "pearl"],
	"fantasy-crew": ["wisp", "draco", "rune", "glimmer", "wish", "keep"],
	"garden-crew": ["prickles", "bonsai", "fern", "acorn", "clover", "berry"],
	"music-crew": ["tempo", "vinyl", "boombox", "strum", "clef", "chime"],
	"adventure-crew": ["scout", "summit", "camp", "beacon", "skipper", "globe"],
	"arcade-crew": ["pixel", "joystick", "dicey", "puzzlo", "marble", "domino"],
	"pirates-crew": ["helm", "coffer", "spyglass", "cask", "anchor", "squawk"],
	"arcane-crew": ["brim", "phial", "broom", "sands", "brew", "emberwick"],
	"celebrations-crew": [
		"noelle",
		"jacklet",
		"paska",
		"confetti",
		"hong",
		"diya",
	],
	"service-crew": ["radio", "hydrant", "rotor", "rucksack", "pylon", "buoy"],
	"workplace-crew": ["brief", "dial", "calc", "type", "badge", "agenda"],
	"healthcare-crew": ["scope", "aid", "capsule", "molar", "pipette", "pulse"],
};

const UNIFORM_PACKS: Record<string, string[]> = {
	"service-crew": [
		"police-cap",
		"police-uniform",
		"fire-helmet",
		"fire-coat",
		"field-cap",
		"field-jacket",
		"rescue-vest",
	],
	"workplace-crew": [
		"office-blazer",
		"reception-vest",
		"office-lanyard",
		"reception-headset",
	],
	"healthcare-crew": ["doctor-coat", "nurse-tunic", "scrub-top", "scrub-cap"],
};

const bundle = (): BotPackDefinition => ({
	id: "favorites",
	label: "Favorites",
	description: "Characters and equipment from several crews.",
	coverKind: "mochi",
	shapeIds: ["mochi", "miso"],
	equipmentIds: ["laptop", "crown"],
});

describe("bot packs", () => {
	it("registers every authored accessory in its existing collection", () => {
		for (const groups of [
			clothingBySeries,
			handPropsBySeries,
			headwearBySeries,
		]) {
			for (const [series, items] of Object.entries(groups)) {
				expect(defaultBotCatalog.series[series]).toBeDefined();
				for (const item of items) {
					expect(defaultBotCatalog.equipment[item.id]).toBeDefined();
					expect(
						defaultBotCatalog.packs[`${series}-crew`].equipmentIds,
					).toContain(item.id);
				}
			}
		}
	});

	it("groups every character into its named pack", () => {
		expect(Object.keys(defaultBotCatalog.packs)).toEqual(
			Object.keys(EXPECTED_PACKS),
		);
		const included = new Set<string>();
		for (const [id, members] of Object.entries(EXPECTED_PACKS)) {
			const pack = defaultBotCatalog.packs[id];
			expect(pack.shapeIds).toEqual(members);
			expect(pack.coverKind).toBe(members[0]);
			expect(
				getBotPackContents(pack, defaultBotCatalog).shapes.map(
					(shape) => shape.id,
				),
			).toEqual(members);
			expect(
				getBotPackContents(pack, defaultBotCatalog).equipment.map(
					(item) => item.id,
				),
			).toEqual([...(pack.equipmentIds ?? [])]);
			expect(pack.equipmentIds).toEqual(
				expect.arrayContaining(UNIFORM_PACKS[id] ?? []),
			);
			expect(pack.equipmentIds?.length).toBeGreaterThan(0);
			for (const member of members) {
				expect(
					getBotShapePacks(member, defaultBotCatalog).map((entry) => entry.id),
				).toEqual([id]);
				included.add(member);
			}
		}
		expect([...included]).toEqual(Object.keys(defaultBotCatalog.shapes));
		for (const equipment of Object.keys(defaultBotCatalog.equipment)) {
			const containingPacks = Object.values(defaultBotCatalog.packs)
				.filter((pack) => pack.equipmentIds?.includes(equipment))
				.map((pack) => pack.id);
			expect(
				getBotEquipmentPacks(equipment, defaultBotCatalog).map(
					(pack) => pack.id,
				),
			).toEqual(containingPacks);
			expect(canUseBotEquipment(equipment, defaultBotCatalog, [])).toBe(
				containingPacks.length === 0,
			);
			for (const id of containingPacks)
				expect(canUseBotEquipment(equipment, defaultBotCatalog, [id])).toBe(
					true,
				);
		}
	});

	it("keeps starter equipment free and assigns each themed item to one collection", () => {
		const starter = new Set([
			"headset",
			"crown",
			"hardhat",
			"graduation",
			"glasses",
			"sunglasses",
			"vr",
			"laptop",
			"magnifier",
			"clipboard",
			"mug",
			"tool",
			"letter",
			"sweater",
			"turtleneck",
			"hoodie",
			"scarf",
			"bowtie",
			"tie",
		]);
		for (const item of Object.values(defaultBotCatalog.equipment)) {
			const packs = getBotEquipmentPacks(item.id, defaultBotCatalog);
			expect(packs).toHaveLength(starter.has(item.id) ? 0 : 1);
			if (packs.length) {
				const unrelatedPack = Object.keys(defaultBotCatalog.packs).find(
					(id) => id !== packs[0].id,
				);
				if (!unrelatedPack) throw new Error("Missing unrelated pack");
				expect(
					canUseBotEquipment(item.id, defaultBotCatalog, [unrelatedPack]),
				).toBe(false);
			}
		}
	});

	it("extends packs with overlapping characters and equipment without changing existing bundles", () => {
		const catalog = createBotCatalog({ packs: [bundle()] });
		expect(getBotShapePacks("mochi", catalog).map((pack) => pack.id)).toEqual([
			"basics-crew",
			"favorites",
		]);
		expect(getBotShapePacks("miso", catalog).map((pack) => pack.id)).toEqual([
			"pets-crew",
			"favorites",
		]);
		expect(
			getBotEquipmentPacks("laptop", catalog).map((pack) => pack.id),
		).toEqual(["favorites"]);
		const contents = getBotPackContents(catalog.packs.favorites, catalog);
		expect(contents.shapes.map((shape) => shape.id)).toEqual(["mochi", "miso"]);
		expect(contents.equipment.map((item) => item.id)).toEqual([
			"laptop",
			"crown",
		]);
		expect(defaultBotCatalog.packs.favorites).toBeUndefined();
		expect(getBotEquipmentPacks("laptop", defaultBotCatalog)).toEqual([]);
	});

	it("keeps existing clients unrestricted when ownership is omitted", () => {
		const catalog = createBotCatalog({ packs: [bundle()] });
		expect(canUseBotShape("mochi", catalog)).toBe(true);
		expect(canUseBotShape("miso", catalog)).toBe(true);
		expect(canUseBotEquipment("laptop", catalog)).toBe(true);
		expect(canUseBotShape("mochi", catalog, [])).toBe(false);
		expect(canUseBotEquipment("laptop", catalog, [])).toBe(false);
	});

	it("unlocks shared content through any owned containing pack", () => {
		const catalog = createBotCatalog({ packs: [bundle()] });
		for (const owned of [
			["basics-crew"],
			["favorites"],
			["favorites", "favorites"],
		]) {
			expect(canUseBotShape("mochi", catalog, owned)).toBe(true);
		}
		expect(canUseBotShape("mochi", catalog, ["pets-crew"])).toBe(false);
		expect(canUseBotShape("miso", catalog, ["favorites"])).toBe(true);
		expect(canUseBotEquipment("laptop", catalog, ["favorites"])).toBe(true);
		expect(canUseBotEquipment("laptop", catalog, ["basics-crew"])).toBe(false);
		expect(canUseBotEquipment("glasses", catalog, [])).toBe(true);
	});

	it("keeps unbundled extension content usable with an explicit empty ownership list", () => {
		const catalog = createBotCatalog({
			shapes: [{ ...defaultBotCatalog.shapes.mochi, id: "custom" }],
			equipment: [{ ...defaultBotCatalog.equipment.crown, id: "custom-hat" }],
		});
		expect(canUseBotShape("custom", catalog, [])).toBe(true);
		expect(canUseBotEquipment("custom-hat", catalog, [])).toBe(true);
		expect(getBotShapePacks("custom", catalog)).toEqual([]);
	});

	it("never treats unknown content or inherited property names as available", () => {
		for (const unknown of ["missing", "toString", "constructor", "__proto__"]) {
			for (const owned of [undefined, [], ["basics-crew"], [unknown]]) {
				expect(canUseBotShape(unknown, defaultBotCatalog, owned)).toBe(false);
				expect(canUseBotEquipment(unknown, defaultBotCatalog, owned)).toBe(
					false,
				);
			}
			expect(getBotShapePacks(unknown, defaultBotCatalog)).toEqual([]);
			expect(getBotEquipmentPacks(unknown, defaultBotCatalog)).toEqual([]);
			expect(canUseBotShape("mochi", defaultBotCatalog, [unknown])).toBe(false);
		}
		const detached = {
			...bundle(),
			shapeIds: ["toString", "mochi"],
			equipmentIds: ["constructor", "laptop"],
		};
		const contents = getBotPackContents(detached, defaultBotCatalog);
		expect(contents.shapes.map((shape) => shape.id)).toEqual(["mochi"]);
		expect(contents.equipment.map((item) => item.id)).toEqual(["laptop"]);
	});

	it("accepts prototype-like IDs when they are registered own entries", () => {
		const catalog = createBotCatalog({
			shapes: [{ ...defaultBotCatalog.shapes.mochi, id: "constructor" }],
			equipment: [{ ...defaultBotCatalog.equipment.crown, id: "toString" }],
			packs: [
				{
					...bundle(),
					id: "__proto__",
					coverKind: "constructor",
					shapeIds: ["constructor"],
					equipmentIds: ["toString"],
				},
			],
		});
		expect(Object.hasOwn(catalog.packs, "__proto__")).toBe(true);
		expect(canUseBotShape("constructor", catalog, [])).toBe(false);
		expect(canUseBotShape("constructor", catalog, ["__proto__"])).toBe(true);
		expect(canUseBotEquipment("toString", catalog, ["__proto__"])).toBe(true);
		expect(
			getBotShapePacks("constructor", catalog).map((pack) => pack.id),
		).toEqual(["__proto__"]);
	});

	it("copies and freezes pack member arrays while leaving authoring inputs mutable", () => {
		const shapeIds = ["mochi", "miso"];
		const equipmentIds = ["laptop"];
		const source = { ...bundle(), shapeIds, equipmentIds };
		const catalog = createBotCatalog({ packs: [source] });
		shapeIds.push("pip");
		equipmentIds.push("crown");
		source.label = "Changed";
		expect(catalog.packs.favorites.shapeIds).toEqual(["mochi", "miso"]);
		expect(catalog.packs.favorites.equipmentIds).toEqual(["laptop"]);
		expect(catalog.packs.favorites.label).toBe("Favorites");
		for (const value of [
			catalog.packs,
			catalog.packs.favorites,
			catalog.packs.favorites.shapeIds,
			catalog.packs.favorites.equipmentIds,
		]) {
			expect(Object.isFrozen(value)).toBe(true);
		}
		for (const value of [source, shapeIds, equipmentIds])
			expect(Object.isFrozen(value)).toBe(false);
	});

	it("allows explicit pack overrides when extending an extended catalog", () => {
		const first = createBotCatalog({ packs: [bundle()] });
		const second = createBotCatalog(
			{
				packs: [
					{
						...bundle(),
						label: "New label",
						shapeIds: ["mochi"],
						equipmentIds: [],
					},
				],
			},
			first,
		);
		expect(second.packs.favorites.label).toBe("New label");
		expect(second.packs.favorites.shapeIds).toEqual(["mochi"]);
		expect(first.packs.favorites.shapeIds).toEqual(["mochi", "miso"]);
		expect(canUseBotEquipment("laptop", second, [])).toBe(true);
	});

	it("resolves pack members declared in the same extension", () => {
		const catalog = createBotCatalog({
			shapes: [{ ...defaultBotCatalog.shapes.mochi, id: "custom" }],
			equipment: [{ ...defaultBotCatalog.equipment.crown, id: "custom-hat" }],
			packs: [
				{
					...bundle(),
					coverKind: "custom",
					shapeIds: ["custom"],
					equipmentIds: ["custom-hat"],
				},
			],
		});
		expect(
			getBotPackContents(catalog.packs.favorites, catalog).shapes[0].id,
		).toBe("custom");
		expect(canUseBotEquipment("custom-hat", catalog, ["favorites"])).toBe(true);
	});

	it("rejects duplicate or empty pack and member IDs", () => {
		expect(() => createBotCatalog({ packs: [bundle(), bundle()] })).toThrow(
			"Duplicate bot pack",
		);
		expect(() =>
			createBotCatalog({ packs: [{ ...bundle(), id: " " }] }),
		).toThrow("nonempty id");
		for (const pack of [
			{ ...bundle(), shapeIds: ["mochi", "mochi"] },
			{ ...bundle(), equipmentIds: ["laptop", "laptop"] },
		])
			expect(() => createBotCatalog({ packs: [pack] })).toThrow("Duplicate");
		for (const pack of [
			{ ...bundle(), shapeIds: [""] },
			{ ...bundle(), equipmentIds: [" "] },
		])
			expect(() => createBotCatalog({ packs: [pack] })).toThrow("nonempty");
	});

	it("rejects empty contents, missing members and covers outside the pack", () => {
		expect(() =>
			createBotCatalog({
				packs: [{ ...bundle(), shapeIds: [], equipmentIds: [] }],
			}),
		).toThrow("must contain content");
		expect(() =>
			createBotCatalog({ packs: [{ ...bundle(), coverKind: "pip" }] }),
		).toThrow("cover shape must belong");
		expect(() =>
			createBotCatalog({ packs: [{ ...bundle(), shapeIds: [] }] }),
		).toThrow("cover shape must belong");
		for (const unknown of ["missing", "toString", "__proto__"]) {
			expect(() =>
				createBotCatalog({ packs: [{ ...bundle(), shapeIds: [unknown] }] }),
			).toThrow("Unknown shape");
			expect(() =>
				createBotCatalog({ packs: [{ ...bundle(), equipmentIds: [unknown] }] }),
			).toThrow("Unknown equipment");
			expect(() =>
				createBotCatalog({ packs: [{ ...bundle(), coverKind: unknown }] }),
			).toThrow("Unknown cover shape");
		}
	});
});
