import { expect, test } from "bun:test";
import { advanceSmoothText } from "./use-smooth-text";

const text = "x".repeat(300);

test("a long gap closes within about a second and never overshoots", () => {
	let shown = 0;
	let elapsed = 0;
	while (shown < text.length && elapsed < 5_000) {
		shown = advanceSmoothText(shown, text, 16);
		elapsed += 16;
		expect(shown).toBeLessThanOrEqual(text.length);
	}
	expect(shown).toBe(text.length);
	expect(elapsed).toBeLessThan(2_000);
});

test("a small gap still moves on every frame and a caught-up text stays put", () => {
	expect(advanceSmoothText(10, "x".repeat(14), 16)).toBeGreaterThan(10);
	expect(advanceSmoothText(14, "x".repeat(14), 16)).toBe(14);
	expect(advanceSmoothText(20, "x".repeat(14), 16)).toBe(14);
});
