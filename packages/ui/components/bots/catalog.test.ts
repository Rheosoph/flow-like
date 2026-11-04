import { describe, expect, it } from "bun:test";
import { resolveBotAppearance, resolveBotKind } from "./appearance";
import { createBotCatalog, defaultBotCatalog } from "./catalog";
import type {
	BotCatalogExtension,
	BotEquipmentDefinition,
	BotShapeDefinition,
} from "./types";

const customShape = (): BotShapeDefinition => ({
	...defaultBotCatalog.shapes.mochi,
	id: "test-pebble",
	label: "Pebble",
	palette: { ...defaultBotCatalog.shapes.mochi.palette },
	anchors: {
		...defaultBotCatalog.shapes.mochi.anchors,
		head: { x: 50, y: 30, width: 40 },
	},
});

const customEquipment = (): BotEquipmentDefinition => ({
	id: "book",
	label: "Book",
	slot: "hand",
	render: () => null,
});

describe("bot catalog", () => {
	it("preserves saved replacements without duplicating them in the catalog", () => {
		for (const [oldKind, kind] of Object.entries({
			chatty: "bubble",
			keel: "helm",
			azimuth: "spyglass",
			kraki: "anchor",
			vellum: "brim",
			lumlet: "broom",
			runelet: "sands",
			tollie: "brew",
		})) {
			expect(defaultBotCatalog.shapes[oldKind]).toBeUndefined();
			expect(
				resolveBotAppearance({ kind: oldKind }, defaultBotCatalog).kind,
			).toBe(kind);
		}
		const custom = createBotCatalog({
			shapes: [{ ...customShape(), id: "chatty" }],
		});
		expect(resolveBotKind("chatty", custom)).toBe("chatty");
		for (const id of ["missing", "toString", "__proto__"])
			expect(resolveBotKind(id, defaultBotCatalog)).toBeUndefined();
		expect(
			resolveBotKind("chatty", { ...defaultBotCatalog, shapes: {} }),
		).toBeUndefined();
	});

	it("ships twenty-three crew series, each equipment slot and seventeen skills", () => {
		expect(Object.keys(defaultBotCatalog.shapes)).toHaveLength(129);
		expect(Object.keys(defaultBotCatalog.series)).toHaveLength(23);
		for (const [series, size] of Object.entries({
			flow: 4,
			basics: 5,
			nature: 5,
			pets: 5,
			zoo: 5,
			tech: 5,
			snacks: 5,
			cosmos: 5,
			pro: 5,
			office: 7,
			workshop: 6,
			ocean: 6,
			fantasy: 6,
			garden: 6,
			music: 6,
			adventure: 6,
			arcade: 6,
			pirates: 6,
			arcane: 6,
			celebrations: 6,
			service: 6,
			workplace: 6,
			healthcare: 6,
		})) {
			expect(
				Object.values(defaultBotCatalog.shapes).filter(
					(shape) => shape.series === series,
				),
			).toHaveLength(size);
		}
		expect(defaultBotCatalog.defaultKind).toBe("mochi");
		expect(Object.keys(defaultBotCatalog.skills)).toHaveLength(17);
		const slots = Object.values(defaultBotCatalog.equipment).reduce<
			Record<string, number>
		>((counts, item) => {
			counts[item.slot] = (counts[item.slot] ?? 0) + 1;
			return counts;
		}, {});
		for (const [slot, minimum] of Object.entries({
			head: 99,
			eyes: 40,
			hand: 130,
			neck: 140,
		}))
			expect(slots[slot]).toBeGreaterThanOrEqual(minimum);
	});

	it("maps hand items to skills and omits the mug at compact sizes", () => {
		const { equipment } = defaultBotCatalog;
		expect(equipment.laptop.compactSkill).toBe("device");
		expect(equipment.magnifier.compactSkill).toBe("search");
		expect(equipment.clipboard.compactSkill).toBe("tasks");
		expect(equipment.tool.compactSkill).toBe("tools");
		expect(equipment.letter.compactSkill).toBe("mail");
		expect(equipment.mug.compactSkill).toBeUndefined();
	});

	it("extends series and rejects characters assigned to missing series", () => {
		const series = {
			id: "custom",
			label: "My crew",
			description: "Custom characters.",
		};
		const catalog = createBotCatalog({
			series: [series],
			shapes: [{ ...customShape(), series: "custom" }],
		});
		expect(catalog.series.custom).toEqual(series);
		expect(Object.isFrozen(catalog.series.custom)).toBe(true);
		expect(Object.isFrozen(series)).toBe(false);
		expect(defaultBotCatalog.series.custom).toBeUndefined();
		expect(() =>
			createBotCatalog({ shapes: [{ ...customShape(), series: "missing" }] }),
		).toThrow("Unknown series");
		expect(() => createBotCatalog({ series: [series, series] })).toThrow(
			"Duplicate bot series",
		);
	});

	it("resolves equipment references to shapes and skills in the same extension", () => {
		const catalog = createBotCatalog({
			defaultKind: "test-pebble",
			shapes: [customShape()],
			skills: [
				{ id: "reading", label: "Reading", color: "#456789", glyph: "R" },
			],
			equipment: [
				{
					...customEquipment(),
					compactSkill: "reading",
					kinds: ["test-pebble"],
				},
			],
		});
		expect(catalog.defaultKind).toBe("test-pebble");
		expect(catalog.equipment.book.kinds).toEqual(["test-pebble"]);
		expect(catalog.equipment.book.compactSkill).toBe("reading");
		expect(catalog.shapes.mochi).toEqual(defaultBotCatalog.shapes.mochi);
		expect(defaultBotCatalog.shapes["test-pebble"]).toBeUndefined();
	});

	it("allows explicit ID overrides without changing the base catalog", () => {
		const catalog = createBotCatalog({
			shapes: [{ ...customShape(), id: "mochi", label: "My Mochi" }],
			equipment: [{ ...customEquipment(), id: "laptop", label: "My laptop" }],
			skills: [{ ...defaultBotCatalog.skills.web, label: "Browser" }],
		});
		expect(catalog.shapes.mochi.label).toBe("My Mochi");
		expect(catalog.equipment.laptop.label).toBe("My laptop");
		expect(catalog.skills.web.label).toBe("Browser");
		expect(defaultBotCatalog.shapes.mochi.label).toBe("Mochi");
		expect(defaultBotCatalog.equipment.laptop.label).toBe("Laptop");
		expect(defaultBotCatalog.skills.web.label).toBe("Web");
	});

	it("can extend an already extended catalog", () => {
		const first = createBotCatalog({
			defaultKind: "test-pebble",
			shapes: [customShape()],
		});
		const second = createBotCatalog({ equipment: [customEquipment()] }, first);
		expect(second.defaultKind).toBe("test-pebble");
		expect(second.shapes["test-pebble"].label).toBe("Pebble");
		expect(second.equipment.book.label).toBe("Book");
		expect(first.equipment.book).toBeUndefined();
	});

	it("copies and freezes nested definitions without freezing source objects", () => {
		const shape = customShape();
		const kinds = ["test-pebble"];
		const equipment = { ...customEquipment(), kinds };
		const catalog = createBotCatalog({
			shapes: [shape],
			equipment: [equipment],
		});
		shape.palette.body = "#000000";
		shape.anchors.head.width = 12;
		kinds.push("mochi");
		equipment.label = "Changed";
		expect(catalog.shapes["test-pebble"].palette.body).toBe(
			defaultBotCatalog.shapes.mochi.palette.body,
		);
		expect(catalog.shapes["test-pebble"].anchors.head.width).toBe(40);
		expect(catalog.equipment.book.kinds).toEqual(["test-pebble"]);
		expect(catalog.equipment.book.label).toBe("Book");
		for (const value of [
			catalog,
			catalog.shapes,
			catalog.equipment,
			catalog.skills,
			catalog.shapes["test-pebble"],
			catalog.shapes["test-pebble"].palette,
			catalog.shapes["test-pebble"].anchors,
			catalog.shapes["test-pebble"].anchors.eyes,
			catalog.shapes["test-pebble"].anchors.eyes.left,
			catalog.shapes["test-pebble"].anchors.head,
			catalog.shapes["test-pebble"].anchors.neck,
			catalog.shapes["test-pebble"].anchors.hand,
			catalog.shapes["test-pebble"].anchors.mouth,
			catalog.equipment.book,
			catalog.equipment.book.kinds,
			catalog.skills.web,
		]) {
			expect(Object.isFrozen(value)).toBe(true);
		}
		expect(Object.isFrozen(shape)).toBe(false);
		expect(Object.isFrozen(shape.palette)).toBe(false);
		expect(Object.isFrozen(kinds)).toBe(false);
	});

	it("rejects duplicate IDs within each extension category", () => {
		const extensions: BotCatalogExtension[] = [
			{ shapes: [customShape(), customShape()] },
			{ equipment: [customEquipment(), customEquipment()] },
			{ skills: [defaultBotCatalog.skills.web, defaultBotCatalog.skills.web] },
		];
		for (const extension of extensions) {
			expect(() => createBotCatalog(extension)).toThrow("Duplicate bot");
		}
	});

	it("copies exposed detail color roles and rejects invalid background relationships", () => {
		const backgroundDetailColors: NonNullable<
			BotShapeDefinition["backgroundDetailColors"]
		>[number][] = ["accent"];
		const catalog = createBotCatalog({
			shapes: [{ ...customShape(), backgroundDetailColors }],
		});
		backgroundDetailColors.push("clothing");
		expect(catalog.shapes["test-pebble"].backgroundDetailColors).toEqual([
			"accent",
		]);
		expect(
			Object.isFrozen(catalog.shapes["test-pebble"].backgroundDetailColors),
		).toBe(true);
		expect(Object.isFrozen(backgroundDetailColors)).toBe(false);
		for (const role of ["background", "not-a-role"])
			expect(() =>
				createBotCatalog({
					shapes: [
						{
							...customShape(),
							backgroundDetailColors: [
								role,
							] as BotShapeDefinition["backgroundDetailColors"],
						},
					],
				}),
			).toThrow("Invalid background detail color");
	});

	it("rejects empty IDs and missing default shapes", () => {
		expect(() =>
			createBotCatalog({ shapes: [{ ...customShape(), id: " " }] }),
		).toThrow("nonempty id");
		for (const defaultKind of ["missing", "", "toString", "__proto__"]) {
			expect(() => createBotCatalog({ defaultKind })).toThrow(
				"Unknown default bot shape",
			);
		}
	});

	it("rejects unknown equipment slots", () => {
		expect(() =>
			createBotCatalog({
				equipment: [
					{
						...customEquipment(),
						slot: "feet" as BotEquipmentDefinition["slot"],
					},
				],
			}),
		).toThrow("Unknown slot");
	});

	it("rejects unavailable compact skills and compact skills on non-hand slots", () => {
		for (const compactSkill of ["missing", "toString", ""]) {
			expect(() =>
				createBotCatalog({
					equipment: [{ ...customEquipment(), compactSkill }],
				}),
			).toThrow("Invalid compact skill");
		}
		expect(() =>
			createBotCatalog({
				equipment: [
					{ ...customEquipment(), slot: "head", compactSkill: "web" },
				],
			}),
		).toThrow("Invalid compact skill");
	});

	it("rejects shape restrictions that cannot resolve", () => {
		for (const kind of ["missing", "toString", "__proto__"]) {
			expect(() =>
				createBotCatalog({
					equipment: [{ ...customEquipment(), kinds: [kind] }],
				}),
			).toThrow("Unknown shape");
		}
	});

	it("copies equipment exclusions and rejects unknown excluded shapes", () => {
		const excludedKinds = ["fern"];
		const catalog = createBotCatalog({
			equipment: [{ ...customEquipment(), excludedKinds }],
		});
		excludedKinds.push("mochi");
		expect(catalog.equipment.book.excludedKinds).toEqual(["fern"]);
		expect(Object.isFrozen(catalog.equipment.book.excludedKinds)).toBe(true);
		expect(Object.isFrozen(excludedKinds)).toBe(false);
		for (const kind of ["missing", "toString", "__proto__"])
			expect(() =>
				createBotCatalog({
					equipment: [{ ...customEquipment(), excludedKinds: [kind] }],
				}),
			).toThrow("Unknown shape");
	});

	it("stores prototype-like IDs as own catalog entries", () => {
		const catalog = createBotCatalog({
			shapes: [{ ...customShape(), id: "__proto__" }],
			defaultKind: "__proto__",
			equipment: [
				{ ...customEquipment(), id: "constructor", kinds: ["__proto__"] },
			],
		});
		expect(Object.hasOwn(catalog.shapes, "__proto__")).toBe(true);
		expect(catalog.shapes.__proto__.label).toBe("Pebble");
		expect(
			Object.getOwnPropertyDescriptor(catalog.equipment, "constructor")?.value,
		).toMatchObject({ label: "Book" });
		expect(Object.getPrototypeOf(catalog.shapes)).toBe(Object.prototype);
	});
});
