import {
	EVENT_CONFIG,
	isChatEventType,
} from "@flow-like/flow-like-ui/lib/event-config";
import { USE_EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config-use";
import { describe, expect, test } from "vitest";

describe("runtime event config", () => {
	// `/use` loads USE_EVENT_CONFIG instead of EVENT_CONFIG so a running app never pulls the
	// builder's configuration panels into its bundle. The two are written by hand, so drift
	// would silently make an event type unrenderable at runtime while it still looks
	// configurable in the editor.
	test("covers exactly the event groups and types the full config declares", () => {
		expect(Object.keys(USE_EVENT_CONFIG).toSorted()).toEqual(
			Object.keys(EVENT_CONFIG).toSorted(),
		);

		for (const [group, full] of Object.entries(EVENT_CONFIG)) {
			expect(USE_EVENT_CONFIG[group].eventTypes).toEqual(full.eventTypes);
			expect(
				Object.keys(USE_EVENT_CONFIG[group].useInterfaces).toSorted(),
			).toEqual(Object.keys(full.useInterfaces).toSorted());
		}
	});

	test("resolves the same interface component for every renderable event type", () => {
		for (const [group, full] of Object.entries(EVENT_CONFIG)) {
			for (const [eventType, component] of Object.entries(full.useInterfaces)) {
				expect(USE_EVENT_CONFIG[group].useInterfaces[eventType]).toBe(
					component,
				);
			}
		}
	});
});

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

describe("chat event config", () => {
	test("offers only the built-in Chat UI as a chat interface", () => {
		expect(EVENT_CONFIG.events_chat.eventTypes).toContain("simple_chat");
		expect(EVENT_CONFIG.events_chat.eventTypes).not.toContain("advanced_chat");
		expect(isChatEventType("simple_chat")).toBe(true);
		expect(isChatEventType("advanced_chat")).toBe(false);
		expect(isChatEventType("chat_stream")).toBe(false);
	});
});

describe("workflow Event entry compatibility", () => {
	test("keeps Simple, Generic, and Chat entry setup separate", () => {
		expect(EVENT_CONFIG.events_simple.defaultEventType).toBe("quick_action");
		expect(EVENT_CONFIG.events_simple.eventTypes).toEqual(
			expect.arrayContaining(["quick_action", "cron", "daemon", "rest", "mcp"]),
		);
		expect(EVENT_CONFIG.events_generic.defaultEventType).toBe("generic_form");
		expect(EVENT_CONFIG.events_generic.eventTypes).toEqual(
			expect.arrayContaining(["generic_form", "api", "deeplink"]),
		);
		expect(EVENT_CONFIG.events_chat.defaultEventType).toBe("simple_chat");
		expect(EVENT_CONFIG.events_chat.eventTypes).toEqual(
			expect.arrayContaining(["simple_chat", "discord", "telegram"]),
		);
		expect(EVENT_CONFIG.events_generic.eventTypes).not.toContain("cron");
		expect(EVENT_CONFIG.events_chat.eventTypes).not.toContain("cron");
	});

	test("cron is serialized as Simple Event sink config", () => {
		const cronConfig = {
			...EVENT_CONFIG.events_simple.configs.cron,
			expression: "0 8 * * *",
			timezone: "Europe/Berlin",
			last_fired: null,
			sink_execution: "LOCAL",
		};
		const encoded = new TextEncoder().encode(JSON.stringify(cronConfig));
		const decoded = JSON.parse(new TextDecoder().decode(encoded));

		expect(decoded).toEqual({
			sink_type: "cron",
			expression: "0 8 * * *",
			timezone: "Europe/Berlin",
			last_fired: null,
			sink_execution: "LOCAL",
		});
	});
});
