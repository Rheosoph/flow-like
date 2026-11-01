import type { BotEffect } from "./types";

export interface BotEffectDefinition {
	id: BotEffect;
	label: string;
	description: string;
}

export const BOT_EFFECT_GROUPS = [
	{
		id: "screen",
		label: "Retro & Screen",
		description: "Old screens and cameras",
		effects: [
			{
				id: "pixel",
				label: "Pixel",
				description: "24 × 24 grid, flat colors, crisp edges",
			},
			{
				id: "lcd",
				label: "LCD",
				description: "Four green tones and a screen grid",
			},
			{
				id: "crt",
				label: "CRT",
				description: "RGB offset, scanlines, and glow",
			},
			{
				id: "glitch",
				label: "Glitch",
				description: "Color shifts and little jumps",
			},
			{
				id: "thermal",
				label: "Thermal",
				description: "Heat colors on a night background",
			},
		],
	},
	{
		id: "print",
		label: "Print & Paper",
		description: "Ink, texture, and handmade edges",
		effects: [
			{ id: "riso", label: "Riso", description: "Three ink tones and grain" },
			{
				id: "raster",
				label: "Raster",
				description: "Dots like newspaper print",
			},
			{
				id: "comic",
				label: "Comic",
				description: "Ink outlines and halftone dots",
			},
			{
				id: "watercolor",
				label: "Watercolor",
				description: "Paper grain and pigment edges",
			},
			{
				id: "blueprint",
				label: "Blueprint",
				description: "Blue tones and a drafting grid",
			},
		],
	},
	{
		id: "pop",
		label: "Pop",
		description: "Toys, stickers, and glowing signs",
		effects: [
			{
				id: "sticker",
				label: "Sticker",
				description: "White outline and a soft shadow",
			},
			{
				id: "vinyl",
				label: "Vinyl",
				description: "Glossy highlights and a toy finish",
			},
			{ id: "neon", label: "Neon", description: "Glowing outlines after dark" },
		],
	},
	{
		id: "status",
		label: "Status",
		description: "The original look and a quieter presence",
		effects: [
			{ id: "none", label: "None", description: "Original colors and shapes" },
			{
				id: "offline",
				label: "Offline",
				description: "Gray and gently dimmed",
			},
		],
	},
] as const satisfies readonly {
	id: string;
	label: string;
	description: string;
	effects: readonly BotEffectDefinition[];
}[];

export const BOT_EFFECTS: readonly BotEffectDefinition[] =
	BOT_EFFECT_GROUPS.flatMap(
		(group) => [...group.effects] as BotEffectDefinition[],
	);
