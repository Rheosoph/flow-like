import { describe, expect, it } from "vitest";
import defaults from "./generated/event-defaults.json";
import { buildNativeEventDefaults } from "./native-event-defaults";

describe("native Event defaults", () => {
	it("matches web Event configuration, including the chat theme and disclosure", () => {
		expect(defaults).toEqual(buildNativeEventDefaults());
	});
});
