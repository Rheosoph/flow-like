import { describe, expect, test } from "bun:test";
import { shouldTranslate } from "./translate";

describe("translation safety classifier", () => {
	test.each([
		"flex flex-col items-center justify-center gap-4 h-full p-4",
		"minmax(240px, 1fr)",
		"brew install ngrok",
		"from flow_like import Flow",
		'<img src="{{url}}" style="max-width: 100%; height: auto;" />',
	])("copies technical values without translating them: %s", (value) => {
		expect(shouldTranslate(value)).toBe(false);
	});

	test.each([
		"Save your changes",
		"{{count}} packages updated",
		"This will disable {{name}} and remove it from search results.",
	])("translates human-facing prose: %s", (value) => {
		expect(shouldTranslate(value)).toBe(true);
	});
});
