import { describe, expect, test } from "vitest";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";

describe("daemon event config", () => {
	test("is available as a local-only simple event sink", () => {
		const simple = EVENT_CONFIG.events_simple;

		expect(simple.eventTypes).toContain("daemon");
		expect(simple.withSink).toContain("daemon");
		expect(simple.sinkAvailability?.daemon?.availability).toBe("local");
		expect(simple.configInterfaces.daemon).toBeDefined();
	});

	test("round-trips default daemon config through event payload bytes", () => {
		const daemonConfig = EVENT_CONFIG.events_simple.configs.daemon;

		const encoded = new TextEncoder().encode(JSON.stringify(daemonConfig));
		const decoded = JSON.parse(new TextDecoder().decode(encoded));

		expect(decoded).toEqual({
			sink_type: "daemon",
			restart_policy: "on_failure",
			min_restart_delay_ms: 1000,
			max_restart_delay_ms: 30000,
			board_poll_interval_ms: 3000,
			log_flush_interval_ms: 5000,
			log_batch_size: 500,
			healthy_reset_ms: 60000,
		});
	});
});
