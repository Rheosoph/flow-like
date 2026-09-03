import { describe, expect, test } from "bun:test";
import {
	mayDispatchRawPageBoardAction,
	serializePageTrigger,
	withCurrentManifestRevision,
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

describe("withCurrentManifestRevision", () => {
	const compiled = {
		kind: "action" as const,
		actionId: "pa1_button",
		manifestRevision: "per1_rendered",
	};

	test("re-stamps a compiled action with the authority's current revision", () => {
		const next = withCurrentManifestRevision(compiled, "per1_current");
		expect(next).not.toBe(compiled);
		expect(next.manifestRevision).toBe("per1_current");
		expect(next.actionId).toBe("pa1_button");
		expect(serializePageTrigger(next)).toEqual({
			kind: "action",
			action_id: "pa1_button",
			manifest_revision: "per1_current",
		});
	});

	test("re-stamps a lifecycle trigger too", () => {
		const special = {
			kind: "special" as const,
			specialEvent: "load" as const,
			manifestRevision: "per1_rendered",
		};
		expect(
			withCurrentManifestRevision(special, "per1_current").manifestRevision,
		).toBe("per1_current");
	});

	// A grant is minted against one exact revision and the authority still
	// compares it. Re-stamping would resurrect a revoked capability.
	test("never re-stamps a dynamic grant", () => {
		const local = {
			kind: "action" as const,
			actionId: "lda1_grant",
			manifestRevision: "per1_rendered",
		};
		const server = {
			kind: "action" as const,
			actionId: "da1_grant",
			manifestRevision: "per1_rendered",
		};
		const bearer = {
			kind: "action" as const,
			actionId: "pa1_looks_compiled",
			capabilityJwt: "token",
			manifestRevision: "per1_rendered",
		};
		expect(withCurrentManifestRevision(local, "per1_current")).toBe(local);
		expect(withCurrentManifestRevision(server, "per1_current")).toBe(server);
		expect(withCurrentManifestRevision(bearer, "per1_current")).toBe(bearer);
	});

	// Carrying a revision at all is what proves the caller came through a real
	// bootstrap; both gates reject an absent one. Minting one here would forge
	// that provenance.
	test("never mints a revision the surface never had", () => {
		const unstamped = { kind: "action" as const, actionId: "pa1_button" };
		expect(withCurrentManifestRevision(unstamped, "per1_current")).toBe(
			unstamped,
		);
		expect(serializePageTrigger(unstamped)).toEqual({
			kind: "action",
			action_id: "pa1_button",
		});
	});

	// serializePageTrigger drops a falsy revision, so blanking one turns a
	// runnable click into a hard refusal.
	test("never blanks an existing revision", () => {
		expect(withCurrentManifestRevision(compiled, undefined)).toBe(compiled);
		expect(withCurrentManifestRevision(compiled, null)).toBe(compiled);
		expect(withCurrentManifestRevision(compiled, "   ")).toBe(compiled);
	});

	test("returns the same reference when already current, so drift is detectable", () => {
		expect(withCurrentManifestRevision(compiled, "per1_rendered")).toBe(
			compiled,
		);
	});
});
