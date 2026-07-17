import { describe, expect, test } from "bun:test";
import { DEFAULT_CHAT_THEME_CSS } from "@flow-like/flow-like-ui";
import { EVENT_CONFIG } from "./event-config";

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
	test("offers Chat UI without the removed advanced chat type", () => {
		expect(EVENT_CONFIG.events_chat.eventTypes).toContain("simple_chat");
		expect(EVENT_CONFIG.events_chat.eventTypes).not.toContain("advanced_chat");
	});

	test("includes safe appearance and AI disclosure defaults", () => {
		const chatConfig = EVENT_CONFIG.events_chat.configs.simple_chat;
		expect(chatConfig).toMatchObject({
			background_image: "",
			custom_css: DEFAULT_CHAT_THEME_CSS,
		});
		expect(chatConfig).not.toHaveProperty("color_scheme");
		expect(chatConfig.ai_disclosure).toContain("AI");
	});
});
