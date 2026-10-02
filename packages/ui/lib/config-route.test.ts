import { describe, expect, test } from "bun:test";
import {
	configRouteFillsHeight,
	configRouteOpensMaximized,
} from "./config-route";

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

describe("configRouteOpensMaximized", () => {
	test("the deploy wizard in the Devices section opens the card maximized", () => {
		expect(configRouteOpensMaximized("/library/config/devices", "deploy")).toBe(
			true,
		);
		expect(
			configRouteOpensMaximized("/library/config/devices/", "deploy"),
		).toBe(true);
	});

	test("every other page and flow keeps the sidebar", () => {
		expect(configRouteOpensMaximized("/library/config/devices", null)).toBe(
			false,
		);
		expect(configRouteOpensMaximized("/library/config/devices", "setup")).toBe(
			false,
		);
		expect(configRouteOpensMaximized("/library/config/events", "deploy")).toBe(
			false,
		);
		expect(configRouteOpensMaximized(null, "deploy")).toBe(false);
	});
});
