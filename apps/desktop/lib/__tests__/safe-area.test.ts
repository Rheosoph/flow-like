import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (relative: string) =>
	readFileSync(new URL(relative, import.meta.url), "utf8");

/**
 * The iOS status bar / home indicator overlap regressions all reduce to one
 * thing: `env(safe-area-inset-*)` silently resolving to 0px. WebKit only
 * populates those when the *server-rendered* viewport meta carries
 * `viewport-fit=cover`, and it does not reliably recompute them when the meta
 * is patched from JS afterwards. Next.js only emits it from a `viewport` export
 * in a Server Component, so both facts have to hold together.
 */
describe("iOS safe-area contract", () => {
	const layout = read("../../app/layout.tsx");

	it("ships viewport-fit=cover in the rendered HTML", () => {
		expect(layout).toMatch(/viewportFit:\s*"cover"/);
	});

	it("keeps the root layout a Server Component so the viewport export applies", () => {
		expect(layout).toMatch(/export const viewport/);
		// Next.js silently ignores `viewport` exports from "use client" modules.
		expect(layout.slice(0, 200)).not.toMatch(/["']use client["']/);
	});

	it("resolves the safe-area vars from both env() and the native bridge", () => {
		const css = read("../../../../packages/ui/global.css");

		for (const [name, envName] of [
			["--fl-safe-top", "safe-area-inset-top"],
			["--fl-safe-bottom", "safe-area-inset-bottom"],
		] as const) {
			const declaration = css.match(
				new RegExp(`${name}:\\s*max\\(([^;]*)\\);`),
			)?.[1];
			expect(declaration, `${name} must be declared`).toBeDefined();
			expect(declaration).toContain(`env(${envName}`);
			expect(declaration).toContain("--fl-native-safe-");
		}
	});

	it("reads real UIKit insets natively instead of only re-probing env()", () => {
		const lib = read("../../src-tauri/src/lib.rs");

		expect(lib).toContain("safeAreaInsets");
		expect(lib).toMatch(/fn probe_ios_native_insets/);
	});
});
