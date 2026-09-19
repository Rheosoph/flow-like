import { describe, expect, it } from "bun:test";
import {
	HOME_ACCENTS,
	homeAppearanceStyle,
	homeWidgetFrameClassName,
} from "./home-appearance";

function luminance(hex: string) {
	const channels = (hex.slice(1).match(/../g) ?? []).map((channel) => {
		const value = Number.parseInt(channel, 16) / 255;
		return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
	});
	return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

describe("home surface colors", () => {
	it("keeps normal-sized text readable on every solid palette color", () => {
		for (const [name, color] of Object.entries(HOME_ACCENTS)) {
			if (name === "neutral") continue;
			const style = homeAppearanceStyle({ variant: "solid", accent: name });
			const foreground = String(
				(style as Record<string, unknown>)["--home-surface-foreground"],
			);
			const ratio = (luminance(color) + 0.05) / (luminance(foreground) + 0.05);
			expect(ratio).toBeGreaterThanOrEqual(4.5);
		}
	});
	it("does not inject arbitrary imported accent values into CSS", () => {
		const style = homeAppearanceStyle({
			variant: "solid",
			accent: "url(https://invalid.example)",
		});
		expect(style).toMatchObject({
			"--home-surface-background": "var(--foreground)",
			"--home-surface-foreground": "var(--background)",
		});
	});
});

describe("home widget frame classes", () => {
	const frame = (
		appearance: Parameters<typeof homeWidgetFrameClassName>[0]["appearance"],
		state: Partial<{
			editing: boolean;
			selected: boolean;
			placeholder: boolean;
			autoHeight: boolean;
		}> = {},
	) =>
		homeWidgetFrameClassName({
			appearance,
			editing: false,
			selected: false,
			placeholder: false,
			autoHeight: true,
			...state,
		}).split(" ");

	it("leaves surface colors overridable instead of inlining them", () => {
		expect(
			homeAppearanceStyle({ variant: "tinted", accent: "rose" }),
		).not.toHaveProperty("backgroundColor");
		expect(
			homeAppearanceStyle({ variant: "solid", accent: "rose" }),
		).not.toHaveProperty("color");
		expect(frame({ variant: "card", accent: "neutral" })).toContain(
			"bg-card/70",
		);
		for (const variant of ["tinted", "solid"]) {
			const classes = frame({ variant, accent: "rose" });
			expect(classes).toContain("bg-[var(--home-surface-background)]");
			expect(classes).not.toContain("bg-card/70");
		}
	});

	it("lets the widget's classes replace conflicting surface classes", () => {
		const classes = frame({
			variant: "tinted",
			accent: "rose",
			className:
				"rounded-none border-primary bg-[#123456] text-white shadow-lg",
		});
		expect(classes).toEqual(
			expect.arrayContaining([
				"rounded-none",
				"border-primary",
				"bg-[#123456]",
				"text-white",
				"shadow-lg",
			]),
		);
		for (const replaced of [
			"rounded-2xl",
			"border-border/60",
			"bg-[var(--home-surface-background)]",
			"text-[var(--home-surface-foreground)]",
			"shadow-sm",
		])
			expect(classes).not.toContain(replaced);
		expect(
			frame(
				{ variant: "card", accent: "neutral", className: "bg-primary/10" },
				{ editing: true },
			),
		).not.toContain("bg-card/80");
	});

	it("keeps editor selection and drop feedback above the widget's classes", () => {
		const selected = frame(
			{ variant: "card", accent: "neutral", className: "ring-4 ring-rose-500" },
			{ editing: true, selected: true },
		);
		expect(selected).toEqual(
			expect.arrayContaining(["ring-2", "ring-primary"]),
		);
		expect(selected).not.toContain("ring-rose-500");
		const placeholder = frame(
			{ variant: "card", accent: "neutral", className: "bg-rose-500" },
			{ editing: true, placeholder: true },
		);
		expect(placeholder).toContain("bg-primary/5");
		expect(placeholder).not.toContain("bg-rose-500");
		expect(
			frame(
				{ variant: "tinted", accent: "rose" },
				{ editing: true, placeholder: true },
			),
		).toContain("bg-[var(--home-surface-background)]");
	});
});
