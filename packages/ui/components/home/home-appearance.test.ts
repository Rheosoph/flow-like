import { describe, expect, it } from "bun:test";
import { HOME_ACCENTS, homeAppearanceStyle } from "./home-appearance";

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
			const ratio =
				(luminance(color) + 0.05) / (luminance(String(style.color)) + 0.05);
			expect(ratio).toBeGreaterThanOrEqual(4.5);
		}
	});
	it("does not inject arbitrary imported accent values into CSS", () => {
		const style = homeAppearanceStyle({
			variant: "solid",
			accent: "url(https://invalid.example)",
		});
		expect(style.backgroundColor).toBe("var(--foreground)");
		expect(style.color).toBe("var(--background)");
	});
});
