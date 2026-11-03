import { defaultBotCatalog } from "./catalog";
import {
	getBotColorBase,
	getBotColorSelectionPaint,
	getBotCustomizableColorSlots,
	normalizeBotColorSelection,
} from "./colors";
import type {
	BotAppearance,
	BotCatalog,
	BotEffect,
	BotPalette,
	BotSkillDefinition,
} from "./types";
import { BOT_COLOR_SLOTS, BOT_EQUIPMENT_SLOTS } from "./types";

export const BOT_OPTIONS = {
	frame: ["circle", "squircle", "square"],
	variant: ["original", "shiny", "midnight", "gold", "pastel", "holo"],
	backdrop: [
		"none",
		"gradient",
		"dots",
		"stripes",
		"waves",
		"rays",
		"stars",
		"confetti",
	],
	halo: ["none", "flow", "gold", "aurora", "pulse", "orbit"],
	effect: [
		"none",
		"pixel",
		"lcd",
		"crt",
		"thermal",
		"riso",
		"glitch",
		"raster",
		"comic",
		"watercolor",
		"blueprint",
		"sticker",
		"vinyl",
		"neon",
		"offline",
	],
	emote: [
		"none",
		"heart",
		"note",
		"spark",
		"alert",
		"question",
		"drop",
		"zzz",
		"rain",
		"anger",
		"loading",
		"idea",
		"check",
		"error",
	],
} as const;

function option<T extends string>(
	value: T | undefined,
	options: readonly T[],
	fallback: T,
): T {
	return value !== undefined && options.includes(value) ? value : fallback;
}

const replacedKinds: Readonly<Record<string, string>> = {
	chatty: "bubble",
	keel: "helm",
	azimuth: "spyglass",
	kraki: "anchor",
	vellum: "brim",
	lumlet: "broom",
	runelet: "sands",
	tollie: "brew",
};

/** Saved characters follow their replacements; explicitly registered IDs take precedence. */
export function resolveBotKind(
	kind: string | undefined,
	catalog: BotCatalog,
): string | undefined {
	if (typeof kind !== "string") return undefined;
	if (Object.hasOwn(catalog.shapes, kind)) return kind;
	const replacement = Object.hasOwn(replacedKinds, kind)
		? replacedKinds[kind]
		: undefined;
	return replacement && Object.hasOwn(catalog.shapes, replacement)
		? replacement
		: undefined;
}

/** Tolerates saved IDs whose extensions are no longer installed. */
export function resolveBotAppearance(
	appearance: Partial<BotAppearance>,
	catalog: BotCatalog,
): Required<BotAppearance> {
	const kind = resolveBotKind(appearance.kind, catalog) ?? catalog.defaultKind;
	const equipment: BotAppearance["equipment"] = {};
	for (const slot of BOT_EQUIPMENT_SLOTS) {
		const id = appearance.equipment?.[slot];
		const item =
			id && Object.hasOwn(catalog.equipment, id)
				? catalog.equipment[id]
				: undefined;
		if (item?.slot === slot && (!item.kinds || item.kinds.includes(kind)))
			equipment[slot] = item.id;
	}
	const colors: NonNullable<BotAppearance["colors"]> = {};
	for (const slot of getBotCustomizableColorSlots(catalog.shapes[kind])) {
		const selection = normalizeBotColorSelection(
			appearance.colors?.[slot],
			catalog,
		);
		if (selection) colors[slot] = selection;
	}
	return {
		kind,
		equipment,
		colors,
		frame: option(appearance.frame, BOT_OPTIONS.frame, "circle"),
		variant: option(appearance.variant, BOT_OPTIONS.variant, "original"),
		backdrop: option(appearance.backdrop, BOT_OPTIONS.backdrop, "none"),
		halo: option(appearance.halo, BOT_OPTIONS.halo, "none"),
		effect: option(appearance.effect, BOT_OPTIONS.effect, "none"),
		emote: option(appearance.emote, BOT_OPTIONS.emote, "none"),
		dark: appearance.dark ?? false,
		mute_notifications: appearance.mute_notifications ?? false,
		skills: [...new Set(appearance.skills ?? [])].filter((id) =>
			Object.hasOwn(catalog.skills, id),
		),
	};
}

export function getBotBadges(
	appearance: Required<BotAppearance>,
	catalog: BotCatalog,
	size: number,
): { visible: BotSkillDefinition[]; overflow: BotSkillDefinition[] } {
	const skills = [...appearance.skills];
	const hand = appearance.equipment.hand
		? catalog.equipment[appearance.equipment.hand]
		: undefined;
	if (size < 56 && hand?.compactSkill && !skills.includes(hand.compactSkill))
		skills.unshift(hand.compactSkill);
	const resolved = skills.flatMap((id) =>
		catalog.skills[id] ? [catalog.skills[id]] : [],
	);
	const capacity = size >= 64 ? 3 : size >= 40 ? 2 : 1;
	// The overflow counter occupies a badge position so the arc never exceeds capacity.
	const count = resolved.length > capacity ? capacity - 1 : capacity;
	return { visible: resolved.slice(0, count), overflow: resolved.slice(count) };
}

export function isBotPaletteEffect(effect: BotEffect): boolean {
	return effect === "lcd" || effect === "blueprint" || effect === "thermal";
}

export function getBotPalette(
	palette: BotPalette,
	appearance: Required<BotAppearance>,
	catalog: BotCatalog = defaultBotCatalog,
): BotPalette {
	const result = { ...palette };
	const shape = Object.hasOwn(catalog.shapes, appearance.kind)
		? catalog.shapes[appearance.kind]
		: undefined;
	for (const slot of shape
		? getBotCustomizableColorSlots(shape)
		: BOT_COLOR_SLOTS) {
		const paint = getBotColorSelectionPaint(appearance.colors?.[slot], catalog);
		if (paint) result[slot] = getBotColorBase(paint);
	}
	if (appearance.dark) result.background = "#232335";
	if (appearance.effect === "lcd")
		return {
			body: "#91a866",
			background: "#d6dfad",
			ink: "#2c4630",
			cheek: "#91a866",
			clothing: "#48643a",
			accent: "#d6dfad",
			sclera: "#d6dfad",
		};
	if (appearance.effect === "blueprint")
		return {
			body: "#3479b8",
			background: "#123e6b",
			ink: "#e5f5ff",
			cheek: "#8bc3e8",
			clothing: "#1e578a",
			accent: "#d4f0ff",
			sclera: "#8fc7e8",
		};
	if (appearance.effect === "thermal")
		return {
			body: "#ff9200",
			background: "#17122d",
			ink: "#5e126d",
			cheek: "#ff36a1",
			clothing: "#ed3974",
			accent: "#fff86d",
			sclera: "#ffe95b",
		};
	if (appearance.effect === "neon") result.background = "#100e1d";
	return result;
}
