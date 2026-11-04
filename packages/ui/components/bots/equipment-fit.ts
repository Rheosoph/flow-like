import type { BotEquipmentDefinition, BotShapeDefinition } from "./types";

/** Keep rendering, selection and saving consistent about which items fit a character. */
export function isBotEquipmentCompatible(
	item: BotEquipmentDefinition,
	shape: BotShapeDefinition,
): boolean {
	return (
		(!item.kinds || item.kinds.includes(shape.id)) &&
		!item.excludedKinds?.includes(shape.id)
	);
}
