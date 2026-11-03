import { resolveBotAppearance, resolveBotKind } from "./appearance";
import {
	canUseBotColor,
	getBotColorPacks,
	getBotCustomizableColorSlots,
	normalizeBotColorSelection,
} from "./colors";
import {
	canUseBotEquipment,
	canUseBotShape,
	getBotEquipmentPacks,
	getBotShapePacks,
} from "./packs";
import {
	BOT_COLOR_SLOTS,
	BOT_EQUIPMENT_SLOTS,
	type BotAppearance,
	type BotCatalog,
	type BotColorSlot,
	type BotEquipmentSlot,
} from "./types";

export interface BotSaveRequirement {
	kind: "shape" | "equipment" | "color";
	id: string;
	label: string;
	slot?: BotEquipmentSlot | BotColorSlot;
	/** Owning any one of these packs grants access. Empty means the selection needs repair. */
	packIds: readonly string[];
}

export interface BotSaveEligibility {
	canSave: boolean;
	requirements: BotSaveRequirement[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function selectionId(value: unknown, fallback: string): string {
	return typeof value === "string" ? value : fallback;
}

/** Checks the persisted selections, including colors hidden by effects. The host must repeat
 * this check with trusted ownership at its persistence boundary. Preview remains unrestricted. */
export function getBotSaveEligibility(
	appearance: Partial<BotAppearance>,
	catalog: BotCatalog,
	ownedPackIds?: readonly string[],
): BotSaveEligibility {
	const requirements: BotSaveRequirement[] = [];
	if (!isRecord(appearance)) {
		return {
			canSave: false,
			requirements: [
				{
					kind: "shape",
					id: "invalid-appearance",
					label: "Invalid appearance",
					packIds: [],
				},
			],
		};
	}
	const requestedKind = appearance.kind;
	const resolved = resolveBotAppearance(
		{ kind: typeof requestedKind === "string" ? requestedKind : undefined },
		catalog,
	);
	const kind =
		requestedKind === undefined
			? resolved.kind
			: (resolveBotKind(requestedKind, catalog) ?? requestedKind);
	const shape =
		typeof kind === "string" && Object.hasOwn(catalog.shapes, kind)
			? catalog.shapes[kind]
			: undefined;
	if (!shape || !canUseBotShape(shape.id, catalog, ownedPackIds)) {
		requirements.push({
			kind: "shape",
			id: selectionId(kind, "invalid-shape"),
			label: shape?.label ?? "Unavailable character",
			packIds: shape
				? getBotShapePacks(shape.id, catalog).map((pack) => pack.id)
				: [],
		});
	}

	const equipment: unknown = appearance.equipment;
	if (equipment !== undefined && !isRecord(equipment)) {
		requirements.push({
			kind: "equipment",
			id: "invalid-equipment",
			label: "Invalid outfit",
			packIds: [],
		});
	} else if (equipment) {
		for (const slot of new Set([
			...BOT_EQUIPMENT_SLOTS,
			...Object.keys(equipment),
		])) {
			const id = equipment[slot];
			if (id === undefined) continue;
			const validSlot = (BOT_EQUIPMENT_SLOTS as readonly string[]).includes(
				slot,
			);
			const item =
				typeof id === "string" && Object.hasOwn(catalog.equipment, id)
					? catalog.equipment[id]
					: undefined;
			const compatible =
				!!item &&
				validSlot &&
				item.slot === slot &&
				(!item.kinds || (!!shape && item.kinds.includes(shape.id)));
			if (compatible && canUseBotEquipment(item.id, catalog, ownedPackIds))
				continue;
			requirements.push({
				kind: "equipment",
				id: selectionId(id, "invalid-equipment"),
				label: item?.label ?? "Unavailable equipment",
				...(validSlot ? { slot: slot as BotEquipmentSlot } : {}),
				packIds: compatible
					? getBotEquipmentPacks(item.id, catalog).map((pack) => pack.id)
					: [],
			});
		}
	}

	const colors: unknown = appearance.colors;
	if (colors !== undefined && !isRecord(colors)) {
		requirements.push({
			kind: "color",
			id: "invalid-colors",
			label: "Invalid colors",
			packIds: [],
		});
	} else if (colors) {
		const supported = shape ? getBotCustomizableColorSlots(shape) : [];
		for (const slot of new Set([...BOT_COLOR_SLOTS, ...Object.keys(colors)])) {
			const value = colors[slot];
			if (value === undefined) continue;
			const validSlot = (BOT_COLOR_SLOTS as readonly string[]).includes(slot);
			const selection = normalizeBotColorSelection(value, catalog);
			const compatible = validSlot && supported.includes(slot as BotColorSlot);
			if (
				selection &&
				compatible &&
				canUseBotColor(selection, catalog, ownedPackIds)
			)
				continue;
			const swatch =
				selection?.kind === "swatch"
					? catalog.colors[selection.colorId]
					: undefined;
			const id =
				selection?.kind === "hex"
					? selection.value
					: (selection?.colorId ??
						(isRecord(value)
							? selectionId(value.colorId, "invalid-color")
							: "invalid-color"));
			requirements.push({
				kind: "color",
				id,
				label:
					swatch?.label ??
					(selection?.kind === "hex"
						? `Custom ${selection.value}`
						: "Invalid color"),
				...(validSlot ? { slot: slot as BotColorSlot } : {}),
				packIds:
					selection && compatible
						? selection.kind === "hex"
							? [selection.packId]
							: getBotColorPacks(selection.colorId, catalog).map(
									(pack) => pack.id,
								)
						: [],
			});
		}
	}
	return { canSave: requirements.length === 0, requirements };
}

export function canSaveBotAppearance(
	appearance: Partial<BotAppearance>,
	catalog: BotCatalog,
	ownedPackIds?: readonly string[],
): boolean {
	return getBotSaveEligibility(appearance, catalog, ownedPackIds).canSave;
}
