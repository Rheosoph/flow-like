import type {
	BotCatalog,
	BotColorPackDefinition,
	BotColorSelection,
	BotColorSlot,
	BotPaint,
	BotShapeDefinition,
} from "./types";
import { BOT_COLOR_SLOTS } from "./types";

export function normalizeBotHex(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const hex = value.trim();
	if (!/^#(?:[\da-f]{3}|[\da-f]{6})$/i.test(hex)) return undefined;
	return (
		hex.length === 4
			? `#${[...hex.slice(1)].map((digit) => digit + digit).join("")}`
			: hex
	).toUpperCase();
}

export function getBotColorBase(paint: BotPaint): string {
	switch (paint.kind) {
		case "solid":
			return normalizeBotHex(paint.color) ?? paint.color;
		case "gradient":
			return normalizeBotHex(paint.colors[0]) ?? paint.colors[0];
		case "pattern":
			return normalizeBotHex(paint.base) ?? paint.base;
	}
}

export function getBotCustomizableColorSlots(
	shape: BotShapeDefinition,
): readonly BotColorSlot[] {
	return shape.colorSlots ?? BOT_COLOR_SLOTS;
}

export function getBotColorPacks(
	colorId: string,
	catalog: BotCatalog,
): BotColorPackDefinition[] {
	if (!Object.hasOwn(catalog.colors ?? {}, colorId)) return [];
	return Object.values(catalog.colorPacks ?? {}).filter((pack) =>
		pack.colorIds.includes(colorId),
	);
}

/** Normalize saved input without checking purchases; existing artwork remains renderable. */
export function normalizeBotColorSelection(
	value: unknown,
	catalog: BotCatalog,
): BotColorSelection | undefined {
	if (!value || typeof value !== "object") return undefined;
	if (!("kind" in value)) return undefined;
	if (
		value.kind === "swatch" &&
		"colorId" in value &&
		typeof value.colorId === "string" &&
		Object.hasOwn(catalog.colors ?? {}, value.colorId)
	)
		return { kind: "swatch", colorId: value.colorId };
	if (
		value.kind !== "hex" ||
		!("value" in value) ||
		!("packId" in value) ||
		typeof value.packId !== "string"
	)
		return undefined;
	const color = normalizeBotHex(value.value);
	const pack = Object.hasOwn(catalog.colorPacks ?? {}, value.packId)
		? catalog.colorPacks[value.packId]
		: undefined;
	return color && pack?.allowsCustomHex
		? { kind: "hex", value: color, packId: pack.id }
		: undefined;
}

export function getBotColorSelectionPaint(
	selection: BotColorSelection | undefined,
	catalog: BotCatalog,
): BotPaint | undefined {
	const normalized = normalizeBotColorSelection(selection, catalog);
	if (!normalized) return undefined;
	return normalized.kind === "hex"
		? { kind: "solid", color: normalized.value }
		: catalog.colors[normalized.colorId].paint;
}

/** Saving requires an included or owned color pack; previewing any known color remains allowed. */
export function canUseBotColor(
	selection: BotColorSelection | undefined,
	catalog: BotCatalog,
	ownedPackIds?: readonly string[],
): boolean {
	const normalized = normalizeBotColorSelection(selection, catalog);
	if (!normalized) return false;
	const packs =
		normalized.kind === "hex"
			? [catalog.colorPacks[normalized.packId]]
			: getBotColorPacks(normalized.colorId, catalog);
	return packs.some(
		(pack) => pack.access === "included" || ownedPackIds?.includes(pack.id),
	);
}
