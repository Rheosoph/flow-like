import { describe, expect, test } from "bun:test";
import {
	mayDispatchRawPageBoardAction,
	serializePageTrigger,
} from "./page-trigger";

describe("Page trigger requests", () => {
	test("keeps a dynamic capability in the request body shape", () => {
		expect(
			serializePageTrigger({
				kind: "action",
				actionId: "da1_runtime",
				capabilityJwt: "signed-capability",
				manifestRevision: "per1_revision",
			}),
		).toEqual({
			kind: "action",
			action_id: "da1_runtime",
			capability_jwt: "signed-capability",
			manifest_revision: "per1_revision",
		});
	});

	test("rejects raw routes on every governed Page", () => {
		expect(mayDispatchRawPageBoardAction(true)).toBe(false);
		expect(mayDispatchRawPageBoardAction(false)).toBe(true);
		expect(mayDispatchRawPageBoardAction(undefined)).toBe(true);
	});
});
