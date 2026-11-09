import { BOT_OPTIONS, resolveBotAppearance } from "./appearance";
import {
	getBotColorSelectionPaint,
	getBotCustomizableColorSlots,
	normalizeBotColorSelection,
} from "./colors";
import { isBotEquipmentCompatible } from "./equipment-fit";
import { canUseBotEquipment, canUseBotShape } from "./packs";
import { randomizeBotColors } from "./random-colors";
import {
	BOT_EQUIPMENT_SLOTS,
	type BotAppearance,
	type BotCatalog,
	type BotEquipmentDefinition,
	type BotEquipmentSlot,
	type BotShapeDefinition,
} from "./types";

export const BOT_LOOK_PARTS = ["crew", "outfit", "colors", "finish"] as const;
export type BotLookPart = (typeof BOT_LOOK_PARTS)[number];

export interface RandomizeBotLookOptions {
	value: BotAppearance;
	catalog: BotCatalog;
	/** Parts to reroll. Locked parts are skipped even when listed here. */
	parts: readonly BotLookPart[];
	/** Parts kept exactly as they are. A rerolled character must fit them. */
	locked?: readonly BotLookPart[];
	ownedPackIds?: readonly string[];
	/** Narrows the characters a character reroll draws from. */
	shapeFilter?: (shape: BotShapeDefinition) => boolean;
	/** Narrows the items an outfit reroll draws from. */
	equipmentFilter?: (item: BotEquipmentDefinition) => boolean;
	random?: () => number;
}

type Equipment = NonNullable<BotAppearance["equipment"]>;
type Colors = NonNullable<BotAppearance["colors"]>;

function fittingEquipment(
	equipment: BotAppearance["equipment"],
	shape: BotShapeDefinition,
	catalog: BotCatalog,
): Equipment {
	const fitting: Equipment = {};
	for (const slot of BOT_EQUIPMENT_SLOTS) {
		const id = equipment?.[slot];
		const item =
			id && Object.hasOwn(catalog.equipment, id)
				? catalog.equipment[id]
				: undefined;
		if (item?.slot === slot && isBotEquipmentCompatible(item, shape))
			fitting[slot] = item.id;
	}
	return fitting;
}

function fittingColors(
	colors: BotAppearance["colors"],
	shape: BotShapeDefinition,
	catalog: BotCatalog,
): Colors {
	const fitting: Colors = {};
	for (const slot of getBotCustomizableColorSlots(shape)) {
		const selection = normalizeBotColorSelection(colors?.[slot], catalog);
		if (selection) fitting[slot] = selection;
	}
	return fitting;
}

/** Rerolls the chosen parts of a look and leaves every other part in place. */
export function randomizeBotLook({
	value,
	catalog,
	parts,
	locked = [],
	ownedPackIds,
	shapeFilter = () => true,
	equipmentFilter = () => true,
	random = Math.random,
}: RandomizeBotLookOptions): BotAppearance {
	const pick = <T>(items: readonly T[]): T =>
		items[Math.min(items.length - 1, Math.floor(random() * items.length))];
	const reroll = new Set(parts.filter((part) => !locked.includes(part)));
	const current = resolveBotAppearance(value, catalog);
	const lockedItems = locked.includes("outfit")
		? Object.values(current.equipment).flatMap((id) =>
				id ? [catalog.equipment[id]] : [],
			)
		: [];
	const lockedColorSlots = locked.includes("colors")
		? Object.keys(current.colors)
		: [];

	let kind = current.kind;
	if (reroll.has("crew")) {
		const usable = Object.values(catalog.shapes).filter((shape) => {
			const slots: readonly string[] = getBotCustomizableColorSlots(shape);
			return (
				canUseBotShape(shape.id, catalog, ownedPackIds) &&
				shapeFilter(shape) &&
				lockedItems.every((item) => isBotEquipmentCompatible(item, shape)) &&
				lockedColorSlots.every((slot) => slots.includes(slot))
			);
		});
		const alternatives = usable.filter((shape) => shape.id !== current.kind);
		const candidates = alternatives.length ? alternatives : usable;
		if (candidates.length) kind = pick(candidates).id;
	}
	const shape = catalog.shapes[kind];
	const changedKind = kind !== current.kind;

	let equipment = changedKind
		? fittingEquipment(value.equipment, shape, catalog)
		: value.equipment;
	if (reroll.has("outfit")) {
		const items = (slot: BotEquipmentSlot) =>
			Object.values(catalog.equipment).filter(
				(item) =>
					item.slot === slot &&
					isBotEquipmentCompatible(item, shape) &&
					canUseBotEquipment(item.id, catalog, ownedPackIds) &&
					equipmentFilter(item),
			);
		const accessorySlots = BOT_EQUIPMENT_SLOTS.filter(
			(slot) => slot !== "neck" && items(slot).length > 0,
		);
		// One outfit and one accessory keep the face readable without crowding the character.
		const chosenSlots: BotEquipmentSlot[] = ["neck"];
		if (accessorySlots.length) chosenSlots.push(pick(accessorySlots));
		const outfit: Equipment = {};
		for (const slot of chosenSlots) {
			const options = items(slot);
			if (options.length && (slot === "neck" || random() >= 0.25))
				outfit[slot] = pick(options).id;
		}
		equipment = outfit;
	}

	const decorated = current.backdrop !== "none" || current.halo !== "none";
	let colors = changedKind
		? fittingColors(value.colors, shape, catalog)
		: value.colors;
	let variant = value.variant;
	if (reroll.has("colors")) {
		colors = randomizeBotColors({
			shape,
			catalog,
			ownedPackIds,
			effect: current.effect,
			// A kept backdrop or halo already carries the look.
			textures: reroll.has("finish") || !decorated,
			random,
		});
		variant = undefined;
	}

	const next: BotAppearance = {
		...value,
		kind,
		equipment,
		colors,
		variant,
	};
	if (reroll.has("finish")) {
		const textured = Object.values(colors ?? {}).some((selection) => {
			const paint = getBotColorSelectionPaint(selection, catalog);
			return paint !== undefined && paint.kind !== "solid";
		});
		// Give a texture, backdrop or halo room to stand on its own.
		const decoration = textured ? 0 : random();
		next.frame = pick(BOT_OPTIONS.frame);
		next.backdrop = decoration >= 0.88 ? pick(BOT_OPTIONS.backdrop) : "none";
		next.halo =
			decoration >= 0.78 && decoration < 0.88 ? pick(BOT_OPTIONS.halo) : "none";
		next.emote = random() < 0.75 ? "none" : pick(BOT_OPTIONS.emote);
		next.dark = undefined;
	}
	return Object.fromEntries(
		Object.entries(next).filter(([, field]) => field !== undefined),
	) as BotAppearance;
}
