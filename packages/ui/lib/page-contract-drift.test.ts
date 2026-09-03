import { beforeEach, describe, expect, test } from "bun:test";
import {
	classifyPageContractError,
	isPageContractDriftFor,
	notifyPageContractRejected,
	resetPageContractDrift,
	subscribeToPageContractDrift,
} from "./page-contract-drift";

beforeEach(() => {
	resetPageContractDrift();
});

describe("classifyPageContractError", () => {
	// Tauri rejects invoke with the SERIALIZED TauriFunctionError, so the native
	// half arrives as a plain `{ error }` object — no Error, no message. Reading
	// only `message` silently kills every native-only failure, which is most of
	// them (the lda1_ gate and the whole local grant registry are native).
	test("reads the Tauri rejection shape", () => {
		expect(
			classifyPageContractError({
				error: "The Page action is stale or invalid",
			}),
		).toBe("stale_action");
		expect(
			classifyPageContractError({
				error: "The local Page action expired; reload the Page",
			}),
		).toBe("dead_grant");
		expect(
			classifyPageContractError({
				error: "The Page manifest is stale; reload the Page",
			}),
		).toBe("stale_manifest");
	});

	test("reads ApiResponseError, Error and bare strings", () => {
		expect(
			classifyPageContractError({
				serverMessage: "The Page action is stale or invalid",
			}),
		).toBe("stale_action");
		expect(
			classifyPageContractError(
				new Error("The Page manifest revision is required"),
			),
		).toBe("stale_manifest");
		expect(
			classifyPageContractError(
				"The local Page action is unknown; reload the Page",
			),
		).toBe("dead_grant");
	});

	// "reload the Page" also ends routing refusals a refetch cannot cure. Matching
	// on it would make every misconfigured Event refetch forever.
	test("does not classify routing refusals that merely say reload", () => {
		expect(
			classifyPageContractError(
				new Error(
					"A local Page action cannot be sent to a Remote Event; reload the Page",
				),
			),
		).toBeNull();
		expect(
			classifyPageContractError(
				new Error(
					"A local Page action cannot be sent to the server; reload the Page",
				),
			),
		).toBeNull();
		expect(
			classifyPageContractError(new Error("Network request failed")),
		).toBeNull();
		expect(classifyPageContractError(undefined)).toBeNull();
	});
});

describe("notifyPageContractRejected", () => {
	test("delivers to subscribers and throttles a repeat of the same failure", () => {
		const seen: string[] = [];
		subscribeToPageContractDrift((detail) => seen.push(detail.reason));

		expect(
			notifyPageContractRejected({
				appId: "app",
				eventId: "evt",
				reason: "stale_action",
			}),
		).toBe(true);
		// A user hammering a dead button must not refetch per press.
		expect(
			notifyPageContractRejected({
				appId: "app",
				eventId: "evt",
				reason: "stale_action",
			}),
		).toBe(false);
		// A different failure on the same Page is a different thing to heal.
		expect(
			notifyPageContractRejected({
				appId: "app",
				eventId: "evt",
				reason: "dead_grant",
			}),
		).toBe(true);
		expect(seen).toEqual(["stale_action", "dead_grant"]);
	});

	test("a throwing subscriber does not stop the others", () => {
		const seen: string[] = [];
		subscribeToPageContractDrift(() => {
			throw new Error("boom");
		});
		subscribeToPageContractDrift((detail) => seen.push(detail.appId));
		notifyPageContractRejected({ appId: "app", reason: "stale_action" });
		expect(seen).toEqual(["app"]);
	});

	test("unsubscribe stops delivery", () => {
		let count = 0;
		const off = subscribeToPageContractDrift(() => count++);
		notifyPageContractRejected({ appId: "app", reason: "stale_action" });
		off();
		notifyPageContractRejected({ appId: "app", reason: "dead_grant" });
		expect(count).toBe(1);
	});
});

describe("isPageContractDriftFor", () => {
	const detail = {
		appId: "app",
		eventId: "evt",
		reason: "stale_action" as const,
	};

	test("matches only its own app and event", () => {
		expect(isPageContractDriftFor(detail, "app", "evt")).toBe(true);
		expect(isPageContractDriftFor(detail, "other", "evt")).toBe(false);
		expect(isPageContractDriftFor(detail, "app", "other-evt")).toBe(false);
	});

	// A receiver whose own Page event has not resolved yet must not match an
	// event-scoped signal, or one Page's failure reloads unrelated surfaces.
	test("an unresolved receiver never matches an event-scoped signal", () => {
		expect(isPageContractDriftFor(detail, "app", null)).toBe(false);
		expect(isPageContractDriftFor(detail, "app", undefined)).toBe(false);
	});

	test("an app-wide signal matches any event of that app", () => {
		const appWide = { appId: "app", reason: "missing_contract" as const };
		expect(isPageContractDriftFor(appWide, "app", "evt")).toBe(true);
		expect(isPageContractDriftFor(appWide, "app", null)).toBe(true);
		expect(isPageContractDriftFor(appWide, "other", "evt")).toBe(false);
	});

	test("an absent app id never matches", () => {
		expect(isPageContractDriftFor(detail, undefined, "evt")).toBe(false);
		expect(isPageContractDriftFor(undefined, "app", "evt")).toBe(false);
	});
});
