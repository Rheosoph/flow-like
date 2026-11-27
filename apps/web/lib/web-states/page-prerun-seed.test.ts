import type { IPrerunEventResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import { describe, expect, it } from "vitest";
import {
	PAGE_LOAD_PRERUN_SEED_MS,
	pagePrerunScope,
	seedPagePrerun,
	takeSeededPagePrerun,
} from "./page-prerun-seed";

const backend = (accessToken: string) => ({
	auth: { user: { access_token: accessToken, profile: { sub: "user-1" } } },
});
const load = {
	kind: "special",
	specialEvent: "load",
	manifestRevision: "per2_a",
} as const;
const response = { signature: "sig" } as IPrerunEventResponse;

describe("bootstrap-seeded page prerun", () => {
	it("serves the exact caller and trigger once while fresh", () => {
		const scope = pagePrerunScope(
			backend("token") as never,
			"app",
			"event",
			undefined,
			load,
		);
		seedPagePrerun(scope, response, 1_000);
		expect(
			takeSeededPagePrerun(scope, 1_000 + PAGE_LOAD_PRERUN_SEED_MS),
		).toEqual({ response, ageMs: PAGE_LOAD_PRERUN_SEED_MS });
		expect(takeSeededPagePrerun(scope, 1_001)).toBeUndefined();
	});

	it("never crosses a token, a revision or a stale seed", () => {
		const scope = pagePrerunScope(
			backend("token") as never,
			"app",
			"event",
			undefined,
			load,
		);
		seedPagePrerun(scope, response, 1_000);
		expect(
			takeSeededPagePrerun(
				pagePrerunScope(
					backend("renewed") as never,
					"app",
					"event",
					undefined,
					load,
				),
				1_001,
			),
		).toBeUndefined();
		expect(
			takeSeededPagePrerun(
				pagePrerunScope(backend("token") as never, "app", "event", undefined, {
					...load,
					manifestRevision: "per2_b",
				}),
				1_001,
			),
		).toBeUndefined();
		expect(
			takeSeededPagePrerun(scope, 1_000 + PAGE_LOAD_PRERUN_SEED_MS + 1),
		).toBeUndefined();
	});
});
