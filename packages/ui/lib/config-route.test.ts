import { describe, expect, test } from "bun:test";
import { configRouteFillsHeight } from "./config-route";

describe("configRouteFillsHeight", () => {
	test("sections that render their own scroll container get a flex slot", () => {
		for (const route of [
			"/library/config/storage",
			"/library/config/user-storage",
			"/library/config/explore",
			"/library/config/setup",
			"/library/config/appearance",
			"/library/config/devices",
		]) {
			expect(configRouteFillsHeight(route)).toBe(true);
		}
	});

	test("ordinary sections scroll the page", () => {
		expect(configRouteFillsHeight("/library/config")).toBe(false);
		expect(configRouteFillsHeight("/library/config/publication")).toBe(false);
		expect(configRouteFillsHeight(null)).toBe(false);
	});
});
