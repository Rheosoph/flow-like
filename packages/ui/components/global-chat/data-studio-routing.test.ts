import { describe, expect, test } from "bun:test";
import {
	DATA_STUDIO_ROUTING_REASONS,
	dataStudioRoutingGate,
} from "./data-studio-routing";

describe("FlowPilot Data Studio routing gate", () => {
	test("preserves build lanes without requiring app inventory", () => {
		expect(
			dataStudioRoutingGate({
				routingReason: "build",
				appInventoryComplete: false,
			}),
		).toBeNull();
	});

	test("requires a completed inventory for every use-time fallback reason", () => {
		for (const routingReason of [
			"explicit_raw_data",
			"no_suitable_event",
			"event_insufficient",
		] as const) {
			const blocked = dataStudioRoutingGate({
				routingReason,
				appInventoryComplete: false,
			});
			expect(blocked).toMatchObject({
				status: "error",
				code: "event_preflight_required",
				routing_reason: routingReason,
				required_tool: "list_apps",
				inventory_complete: false,
				retryable: true,
			});
			expect(
				dataStudioRoutingGate({
					routingReason,
					appInventoryComplete: true,
				}),
			).toBeNull();
		}
	});

	test("fails closed for missing and unknown routing reasons", () => {
		for (const routingReason of [undefined, null, "", "   "]) {
			expect(
				dataStudioRoutingGate({
					routingReason,
					appInventoryComplete: true,
				}),
			).toMatchObject({
				code: "data_studio_routing_reason_required",
				required_argument: "routing_reason",
				allowed_values: DATA_STUDIO_ROUTING_REASONS,
			});
		}

		expect(
			dataStudioRoutingGate({
				routingReason: "direct_query",
				appInventoryComplete: true,
			}),
		).toMatchObject({
			code: "data_studio_routing_reason_invalid",
			routing_reason: "direct_query",
			required_argument: "routing_reason",
			allowed_values: DATA_STUDIO_ROUTING_REASONS,
		});
	});

	test("normalizes surrounding whitespace without widening the enum", () => {
		expect(
			dataStudioRoutingGate({
				routingReason: "  explicit_raw_data  ",
				appInventoryComplete: true,
			}),
		).toBeNull();
	});
});
