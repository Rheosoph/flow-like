import type { BotColorDefinition, BotColorPackDefinition } from "./types";

const essentials: readonly BotColorDefinition[] = [
	{
		id: "color-cream",
		label: "Cream",
		paint: { kind: "solid", color: "#FFF8E9" },
	},
	{ id: "color-ink", label: "Ink", paint: { kind: "solid", color: "#30343F" } },
	{
		id: "color-coral",
		label: "Coral",
		paint: { kind: "solid", color: "#F18C80" },
	},
	{
		id: "color-honey",
		label: "Honey",
		paint: { kind: "solid", color: "#F4C762" },
	},
	{
		id: "color-leaf",
		label: "Leaf",
		paint: { kind: "solid", color: "#83B88F" },
	},
	{ id: "color-sky", label: "Sky", paint: { kind: "solid", color: "#72BCE4" } },
	{
		id: "color-lavender",
		label: "Lavender",
		paint: { kind: "solid", color: "#A795E1" },
	},
	{
		id: "color-pink",
		label: "Pink",
		paint: { kind: "solid", color: "#ECA4C1" },
	},
];
const pastels: readonly BotColorDefinition[] = [
	{
		id: "pastel-peach",
		label: "Peach milk",
		paint: { kind: "solid", color: "#FFD3B9" },
	},
	{
		id: "pastel-mint",
		label: "Soft mint",
		paint: { kind: "solid", color: "#C4E8D2" },
	},
	{
		id: "pastel-cloud",
		label: "Cloud blue",
		paint: { kind: "solid", color: "#C7E2F6" },
	},
	{
		id: "pastel-lilac",
		label: "Pale lilac",
		paint: { kind: "solid", color: "#D9CCF0" },
	},
	{
		id: "pastel-blush",
		label: "Blush",
		paint: { kind: "solid", color: "#F6CBDF" },
	},
	{
		id: "pastel-lemon",
		label: "Lemon cream",
		paint: { kind: "solid", color: "#F8EDB4" },
	},
];
const jewels: readonly BotColorDefinition[] = [
	{
		id: "jewel-ruby",
		label: "Ruby",
		paint: { kind: "solid", color: "#A92D56" },
	},
	{
		id: "jewel-sapphire",
		label: "Sapphire",
		paint: { kind: "solid", color: "#355CAF" },
	},
	{
		id: "jewel-emerald",
		label: "Emerald",
		paint: { kind: "solid", color: "#248367" },
	},
	{
		id: "jewel-amethyst",
		label: "Amethyst",
		paint: { kind: "solid", color: "#8451B5" },
	},
	{
		id: "jewel-amber",
		label: "Amber",
		paint: { kind: "solid", color: "#D78C25" },
	},
	{
		id: "jewel-obsidian",
		label: "Obsidian",
		paint: { kind: "solid", color: "#35303F" },
	},
];
const holographic: readonly BotColorDefinition[] = [
	{
		id: "holo-aurora",
		label: "Aurora",
		paint: {
			kind: "gradient",
			colors: ["#68DDD2", "#A89BEF", "#F2A6D5"],
			angle: 45,
			animated: true,
		},
	},
	{
		id: "holo-sunset",
		label: "Sunset",
		paint: {
			kind: "gradient",
			colors: ["#F4CF71", "#F29A98", "#BD91D7"],
			angle: 90,
			animated: true,
		},
	},
	{
		id: "holo-ocean",
		label: "Ocean",
		paint: {
			kind: "gradient",
			colors: ["#72D8AD", "#6ABBDD", "#8B92E3"],
			angle: 0,
			animated: true,
		},
	},
	{
		id: "holo-prism",
		label: "Prism",
		paint: {
			kind: "gradient",
			colors: ["#F3AFD2", "#A5BAF1", "#91E1D2", "#F5D68D"],
			angle: 135,
			animated: true,
		},
	},
];
const patterns: readonly BotColorDefinition[] = [
	{
		id: "pattern-dots",
		label: "Polka dots",
		paint: {
			kind: "pattern",
			base: "#F8DBE8",
			ink: "#C974A0",
			motif: "dots",
			scale: 10,
		},
	},
	{
		id: "pattern-stripes",
		label: "Candy stripes",
		paint: {
			kind: "pattern",
			base: "#FFF1D9",
			ink: "#EE9C8D",
			motif: "stripes",
			scale: 10,
		},
	},
	{
		id: "pattern-stars",
		label: "Starry night",
		paint: {
			kind: "pattern",
			base: "#655C9E",
			ink: "#F4DA8E",
			motif: "stars",
			scale: 14,
		},
	},
	{
		id: "pattern-diamonds",
		label: "Diamonds",
		paint: {
			kind: "pattern",
			base: "#D4EBE4",
			ink: "#66A99A",
			motif: "diamonds",
			scale: 12,
		},
	},
	{
		id: "pattern-hearts",
		label: "Sweethearts",
		paint: {
			kind: "pattern",
			base: "#FCE1DF",
			ink: "#D9788C",
			motif: "hearts",
			scale: 14,
		},
	},
];

export const defaultBotColors: readonly BotColorDefinition[] = [
	...essentials,
	...pastels,
	...jewels,
	...holographic,
	...patterns,
];

export const defaultBotColorPacks: readonly BotColorPackDefinition[] = [
	{
		id: "colors-essentials",
		label: "Essentials",
		description: "Everyday colors for every character.",
		colorIds: essentials.map((color) => color.id),
		access: "included",
	},
	{
		id: "colors-pastels",
		label: "Pastels",
		description: "Soft colors for a gentler look.",
		colorIds: pastels.map((color) => color.id),
		access: "premium",
	},
	{
		id: "colors-jewels",
		label: "Jewel tones",
		description: "Deep colors inspired by gemstones.",
		colorIds: jewels.map((color) => color.id),
		access: "premium",
	},
	{
		id: "colors-holographic",
		label: "Holographic",
		description: "Gradients that slowly shift through their colors.",
		colorIds: holographic.map((color) => color.id),
		access: "premium",
	},
	{
		id: "colors-patterns",
		label: "Patterns",
		description: "Dots, stripes and little details to wear.",
		colorIds: patterns.map((color) => color.id),
		access: "premium",
	},
	{
		id: "color-lab",
		label: "Color lab",
		description: "Choose your own solid colors with hex codes.",
		colorIds: [],
		access: "premium",
		allowsCustomHex: true,
	},
];
