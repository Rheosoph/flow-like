import { describe, expect, test } from "bun:test";
import type { A2UIServerMessage } from "../a2ui/types";
import { shouldRevealProgressively } from "./progressive-page-reveal";

describe("progressive page reveal", () => {
	test("reveals on the first additive render update", () => {
		for (const message of [
			{
				type: "surfaceUpdate",
				surfaceId: "page",
				components: [{ id: "card", component: { type: "card" } }],
			},
			{
				type: "createElement",
				surfaceId: "page",
				parentId: "root",
				component: { id: "card", component: { type: "card" } },
			},
			{
				type: "upsertElement",
				element_id: "title",
				value: { type: "setText", text: "Fresh" },
			},
			{
				type: "dataModelUpdate",
				surfaceId: "page",
				contents: [{ path: "status", value: "ready" }],
			},
		] as A2UIServerMessage[]) {
			expect(shouldRevealProgressively(message)).toBe(true);
		}
	});

	test("keeps waiting for empty, request, navigation, and removal messages", () => {
		for (const message of [
			{ type: "surfaceUpdate", surfaceId: "page", components: [] },
			{ type: "dataModelUpdate", surfaceId: "page", contents: [] },
			{ type: "removeElement", surfaceId: "page", elementId: "old" },
			{ type: "navigateTo", route: "/next", replace: false },
			{
				type: "requestElements",
				requestId: "request",
				selectors: [],
				timeoutMs: 1000,
			},
		] as A2UIServerMessage[]) {
			expect(shouldRevealProgressively(message)).toBe(false);
		}
	});
});
