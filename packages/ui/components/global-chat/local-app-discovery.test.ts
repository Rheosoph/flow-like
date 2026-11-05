import { describe, expect, test } from "bun:test";
import {
	httpRequestRefusal,
	isReturnedAppInventory,
	sealedResearchRefusal,
} from "./local-app-discovery";

const RUN = "run-1";
const state = (
	overrides: Partial<Parameters<typeof sealedResearchRefusal>[1]> = {},
) => ({
	appInventoryReturned: false,
	...overrides,
});
const code = (...args: Parameters<typeof sealedResearchRefusal>) =>
	sealedResearchRefusal(...args)?.code;

describe("isReturnedAppInventory", () => {
	test("a clean inventory opens the public fallback", () => {
		expect(
			isReturnedAppInventory({ status: "ok", complete: true, apps: [] }),
		).toBe(true);
	});

	test("a partial inventory still counts as discovery", () => {
		const partials = [
			{ status: "partial", complete: false, apps: [] },
			{ status: "partial", complete: false, truncated: true, apps: [] },
			{
				status: "partial",
				complete: false,
				missing_profile_app_count: 2,
				apps: [{ app_id: "a", events_status: "error" }],
			},
		];
		for (const inventory of partials) {
			expect(isReturnedAppInventory(inventory)).toBe(true);
		}
	});

	test("a failed or absent listing keeps the fallback shut", () => {
		expect(isReturnedAppInventory({ status: "error", message: "no" })).toBe(
			false,
		);
		expect(isReturnedAppInventory({ status: "timeout" })).toBe(false);
		expect(isReturnedAppInventory({ apps: [] })).toBe(false);
		expect(isReturnedAppInventory(undefined)).toBe(false);
		expect(isReturnedAppInventory(null)).toBe(false);
		expect(isReturnedAppInventory("ok")).toBe(false);
	});
});

describe("sealedResearchRefusal", () => {
	test("a fresh public request must wait for app discovery", () => {
		expect(code(RUN, undefined)).toBe("local_app_discovery_required");
		expect(code(RUN, state())).toBe("local_app_discovery_required");
		expect(sealedResearchRefusal(RUN, state())?.retryable).toBe(true);
	});

	test("a returned inventory allows research", () => {
		expect(code(RUN, state({ appInventoryReturned: true }))).toBeUndefined();
	});

	test("allows the bot to refine research in the same discovered run", () => {
		const discovered = state({ appInventoryReturned: true });
		expect(code(RUN, discovered)).toBeUndefined();
		expect(code(RUN, discovered)).toBeUndefined();
	});

	test("a run with no id is refused so its research budget stays enforceable", () => {
		expect(code(undefined, state())).toBe("sealed_research_unavailable");
		expect(code("  ", state())).toBe("sealed_research_unavailable");
	});
});

describe("httpRequestRefusal", () => {
	test("requires app discovery before an HTTP approval can be requested", () => {
		for (const routingState of [undefined, state()]) {
			const refusal = httpRequestRefusal(RUN, routingState);
			expect(refusal?.code).toBe("local_app_discovery_required");
			expect(refusal?.retryable).toBe(true);
		}
	});

	test("has no one-shot limit after discovery, including after research", () => {
		const discovered = state({
			appInventoryReturned: true,
		});
		expect(httpRequestRefusal(RUN, discovered)).toBeUndefined();
		expect(httpRequestRefusal(RUN, discovered)).toBeUndefined();
	});

	test("requires a run id to establish discovery", () => {
		const discovered = state({ appInventoryReturned: true });
		expect(httpRequestRefusal(undefined, discovered)?.code).toBe(
			"local_app_discovery_required",
		);
		expect(httpRequestRefusal("  ", discovered)?.code).toBe(
			"local_app_discovery_required",
		);
	});
});

describe("research and HTTP fallback discovery", () => {
	test.each([
		["ok", true],
		["partial", true],
		["error", false],
		["timeout", false],
		["cancelled", false],
	] as const)("a %s inventory controls both fallbacks", (status, allowed) => {
		const routingState = state({
			appInventoryReturned: isReturnedAppInventory({ status, apps: [] }),
		});
		for (const gate of [sealedResearchRefusal, httpRequestRefusal]) {
			if (allowed) expect(gate(RUN, routingState)).toBeUndefined();
			else
				expect(gate(RUN, routingState)?.code).toBe(
					"local_app_discovery_required",
				);
		}
	});
});
