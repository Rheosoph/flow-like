import { describe, expect, it } from "bun:test";
import { safeScopedCss } from "../../lib/css-utils";
import {
	HOME_ACCENTS,
	HOME_PLACEHOLDER_CLASS_LIST,
	homeAppearanceStyle,
	homeWidgetCssScope,
	homeWidgetFrameClassName,
	localizeHomeWidgetKeyframes,
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

	it("never colors the widget's own shadow", () => {
		for (const variant of ["card", "tinted", "solid", "borderless"])
			expect(
				frame({
					variant,
					accent: "rose",
					className: "shadow-[0_12px_32px_-16px_rgb(0_0_0/0.35)]",
				}).filter((name) => name.startsWith("shadow-")),
			).toEqual(["shadow-[0_12px_32px_-16px_rgb(0_0_0/0.35)]"]);
	});

	it("keeps editor selection and drop feedback above the widget's classes", () => {
		const selected = frame(
			{ variant: "card", accent: "neutral", className: "ring-4 ring-rose-500" },
			{ editing: true, selected: true },
		);
		expect(selected).toEqual(
			expect.arrayContaining(["ring-2!", "ring-primary!"]),
		);
		const placeholder = frame(
			{ variant: "card", accent: "neutral", className: "bg-rose-500" },
			{ editing: true, placeholder: true },
		);
		expect(placeholder).toContain("bg-primary/5!");
		expect(placeholder).toEqual(
			expect.arrayContaining(HOME_PLACEHOLDER_CLASS_LIST),
		);
		expect(
			frame(
				{ variant: "tinted", accent: "rose" },
				{ editing: true, placeholder: true },
			),
		).toContain("bg-[var(--home-surface-background)]");
	});
});

describe("homeWidgetCssScope", () => {
	it("selects only its widget, with :root meaning the widget surface", () => {
		const scope = homeWidgetCssScope("hero");
		const css = safeScopedCss(
			":root { color: red; } h2 { font-size: 2rem; }",
			scope,
			{
				scopeRoot: true,
			},
		);
		expect(css).toContain('[data-home-widget-style="hero"] {');
		expect(css).toContain('[data-home-widget-style="hero"] h2 {');
		expect(css).not.toContain(":root");
	});

	it("keeps ids with replacement patterns inside the attribute selector", () => {
		for (const id of ["a$'", "a$`", "a$&", "a$$"]) {
			const css = safeScopedCss(
				':root"]{}nav{display:none}[x="{ color: red; } :root { color: blue; }',
				homeWidgetCssScope(id),
				{ scopeRoot: true },
			);
			expect(css).toContain(
				`[data-home-widget-style="${id}"] { color: blue; }`,
			);
			expect(css.startsWith(`[data-home-widget-style="${id}"]"]`)).toBe(true);
		}
	});

	it("escapes ids that would otherwise break out of the attribute selector", () => {
		const scope = homeWidgetCssScope('a"] body, [x="\\\n');
		expect(scope).toBe('[data-home-widget-style="a\\"] body, [x=\\"\\\\\\a "]');
	});
});

describe("localizeHomeWidgetKeyframes", () => {
	it("renames the widget's own keyframes and their uses, leaving app animations alone", () => {
		const css = localizeHomeWidgetKeyframes(
			"@keyframes pulse { to { opacity: 0.5; } } :root { animation: pulse 2s ease-in-out infinite, spin 1s; } h2 { animation-name: pulse, fade; }",
			"hero",
		);
		const local = css.match(/@keyframes (pulse-hw[0-9a-z]+)/)?.[1];
		expect(local).toBeDefined();
		expect(css).toContain(
			`animation: ${local} 2s ease-in-out infinite, spin 1s;`,
		);
		expect(css).toContain(`animation-name: ${local}, fade;`);
		expect(
			localizeHomeWidgetKeyframes("@keyframes pulse {}", "other"),
		).not.toContain(local ?? "");
	});

	it("returns CSS without keyframes or with unparseable syntax unchanged", () => {
		for (const css of [":root { color: red; }", ":root { color: red; "]) {
			expect(localizeHomeWidgetKeyframes(css, "hero")).toBe(css);
		}
	});
});
