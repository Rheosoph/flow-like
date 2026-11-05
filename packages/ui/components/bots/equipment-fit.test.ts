import { describe, expect, it } from "bun:test";
import { resolveBotAppearance } from "./appearance";
import { createBotCatalog, defaultBotCatalog } from "./catalog";
import { isBotEquipmentCompatible } from "./equipment-fit";

describe("bot equipment compatibility", () => {
	it("lets exclusions override supported shapes without limiting other characters", () => {
		const { mochi, fern, miso } = defaultBotCatalog.shapes;
		const item = {
			...defaultBotCatalog.equipment.crown,
			kinds: [mochi.id, fern.id],
			excludedKinds: [fern.id],
		};
		expect(isBotEquipmentCompatible(item, mochi)).toBe(true);
		expect(isBotEquipmentCompatible(item, fern)).toBe(false);
		expect(isBotEquipmentCompatible(item, miso)).toBe(false);
		expect(isBotEquipmentCompatible({ ...item, kinds: undefined }, miso)).toBe(
			true,
		);
	});

	it("omits excluded saved equipment while preserving compatible slots", () => {
		const catalog = createBotCatalog({
			equipment: [
				{ ...defaultBotCatalog.equipment.crown, excludedKinds: ["fern"] },
			],
		});
		const equipment = { head: "crown", neck: "scarf" };
		expect(
			resolveBotAppearance({ kind: "fern", equipment }, catalog).equipment,
		).toEqual({ neck: "scarf" });
		expect(
			resolveBotAppearance({ kind: "mochi", equipment }, catalog).equipment,
		).toEqual(equipment);
		expect(equipment).toEqual({ head: "crown", neck: "scarf" });
	});

	it("keeps every default item available on at least one character", () => {
		const shapes = Object.values(defaultBotCatalog.shapes);
		const unreachable = Object.values(defaultBotCatalog.equipment)
			.filter(
				(item) =>
					!shapes.some((shape) => isBotEquipmentCompatible(item, shape)),
			)
			.map((item) => item.id);
		expect(unreachable).toEqual([]);
	});
});
