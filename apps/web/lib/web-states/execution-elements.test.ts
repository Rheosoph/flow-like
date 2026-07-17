import { describe, expect, test } from "bun:test";
import { executionElementsFromResponse } from "./execution-elements";

describe("executionElementsFromResponse", () => {
	test("unwraps the API response envelope", () => {
		const element = { id: "upload", component: { type: "fileInput" } };

		expect(
			executionElementsFromResponse({
				elements: { "page/upload": element },
			}),
		).toEqual({ "page/upload": element });
	});

	test("accepts a legacy unwrapped element map", () => {
		const elements = {
			"page/upload": { id: "upload", component: { type: "fileInput" } },
		};

		expect(executionElementsFromResponse(elements)).toEqual(elements);
	});

	test("rejects a malformed envelope", () => {
		expect(executionElementsFromResponse({ elements: null })).toEqual({});
	});
});
