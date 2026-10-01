import { type ClassValue, clsx } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

/**
 * The only tone → class map in the devices area (plan §2.7). Components pick a
 * tone; they never compose `text-*`/`bg-*`/`border-*` status classes by hand.
 */
export const TONES = [
	"good",
	"warning",
	"critical",
	"info",
	"unknown",
	"paused",
	"locked",
] as const;
export type Tone = (typeof TONES)[number];
export type ChipTone = Tone | "outline";

export const TONE_TEXT: Record<Tone, string> = {
	good: "text-good",
	warning: "text-warning",
	critical: "text-critical",
	info: "text-info",
	unknown: "text-unknown",
	paused: "text-paused",
	locked: "text-locked",
};

export const TONE_BG: Record<Tone, string> = {
	good: "bg-good-bg",
	warning: "bg-warning-bg",
	critical: "bg-critical-bg",
	info: "bg-info-bg",
	unknown: "bg-unknown-bg",
	paused: "bg-paused-bg",
	locked: "bg-locked-bg",
};

export const TONE_LINE: Record<Tone, string> = {
	good: "border-good-line",
	warning: "border-warning-line",
	critical: "border-critical-line",
	info: "border-info-line",
	unknown: "border-unknown-line",
	paused: "border-paused-line",
	locked: "border-locked-line",
};

/** Graphic fills (bars, dots, segments). Never behind text. */
export const TONE_SOLID: Record<Tone, string> = {
	good: "bg-good-solid",
	warning: "bg-warning-solid",
	critical: "bg-critical-solid",
	info: "bg-info-solid",
	unknown: "bg-unknown-solid",
	paused: "bg-paused-solid",
	locked: "bg-locked-solid",
};

/** The solid colour as `currentColor`, for SVG glyphs and ticks. */
export const TONE_SOLID_TEXT: Record<Tone, string> = {
	good: "text-good-solid",
	warning: "text-warning-solid",
	critical: "text-critical-solid",
	info: "text-info-solid",
	unknown: "text-unknown-solid",
	paused: "text-paused-solid",
	locked: "text-locked-solid",
};

/** Chip/badge surface: soft fill + ink + line. Critical is square-cornered, unknown is dashed (R14). */
export const TONE_CHIP: Record<ChipTone, string> = {
	good: "bg-good-bg text-good border-good-line rounded-full",
	warning: "bg-warning-bg text-warning border-warning-line rounded-full",
	critical: "bg-critical-bg text-critical border-critical-line rounded-sm",
	info: "bg-info-bg text-info border-info-line rounded-full",
	unknown:
		"bg-unknown-bg text-unknown border-unknown-line border-dashed rounded-full",
	paused: "bg-paused-bg text-paused border-paused-line rounded-full",
	locked: "bg-locked-bg text-locked border-locked-line rounded-full",
	outline: "bg-transparent text-ink-2 border-border rounded-full",
};

/** Boxed surfaces (banners, inline results): soft fill + line, text stays foreground. */
export const TONE_SURFACE: Record<Tone, string> = {
	good: "bg-good-bg border-good-line",
	warning: "bg-warning-bg border-warning-line",
	critical: "bg-critical-bg border-critical-line",
	info: "bg-info-bg border-info-line",
	unknown: "bg-unknown-bg border-unknown-line border-dashed",
	paused: "bg-paused-bg border-paused-line",
	locked: "bg-locked-bg border-locked-line",
};

const merge = extendTailwindMerge({
	extend: {
		classGroups: {
			"font-size": [{ text: ["ui", "label", "headline"] }],
		},
	},
});

/**
 * `cn` for the devices area. The stock merger reads `text-ui`/`text-label`/
 * `text-headline` as colours and drops them next to `text-good` etc.; this one
 * knows they are font sizes. Use it whenever those size tokens are combined.
 */
export function cx(...inputs: ClassValue[]) {
	return merge(clsx(inputs));
}
