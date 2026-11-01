import type {
	BotCatalog,
	BotEquipmentDefinition,
	BotPackDefinition,
	BotShapeDefinition,
} from "./types";

export interface BotPackContents {
	shapes: BotShapeDefinition[];
	equipment: BotEquipmentDefinition[];
}

export function getBotPackContents(
	pack: BotPackDefinition,
	catalog: BotCatalog,
): BotPackContents {
	return {
		shapes: pack.shapeIds.flatMap((id) =>
			Object.hasOwn(catalog.shapes, id) ? [catalog.shapes[id]] : [],
		),
		equipment: (pack.equipmentIds ?? []).flatMap((id) =>
			Object.hasOwn(catalog.equipment, id) ? [catalog.equipment[id]] : [],
		),
	};
}

export function getBotShapePacks(
	kind: string,
	catalog: BotCatalog,
): BotPackDefinition[] {
	if (!Object.hasOwn(catalog.shapes, kind)) return [];
	return Object.values(catalog.packs).filter((pack) =>
		pack.shapeIds.includes(kind),
	);
}

export function getBotEquipmentPacks(
	id: string,
	catalog: BotCatalog,
): BotPackDefinition[] {
	if (!Object.hasOwn(catalog.equipment, id)) return [];
	return Object.values(catalog.packs).filter((pack) =>
		pack.equipmentIds?.includes(id),
	);
}

/** Save entitlement only; previews may use any catalog item. The host enforces purchased access. */
export function canUseBotShape(
	kind: string,
	catalog: BotCatalog,
	ownedPackIds?: readonly string[],
): boolean {
	if (!Object.hasOwn(catalog.shapes, kind)) return false;
	if (ownedPackIds === undefined) return true;
	if (!getBotShapePacks(kind, catalog).length) return true;
	return ownedPackIds.some(
		(id) =>
			Object.hasOwn(catalog.packs, id) &&
			catalog.packs[id].shapeIds.includes(kind),
	);
}

export function canUseBotEquipment(
	id: string,
	catalog: BotCatalog,
	ownedPackIds?: readonly string[],
): boolean {
	if (!Object.hasOwn(catalog.equipment, id)) return false;
	if (ownedPackIds === undefined) return true;
	if (!getBotEquipmentPacks(id, catalog).length) return true;
	return ownedPackIds.some(
		(packId) =>
			Object.hasOwn(catalog.packs, packId) &&
			!!catalog.packs[packId].equipmentIds?.includes(id),
	);
}
