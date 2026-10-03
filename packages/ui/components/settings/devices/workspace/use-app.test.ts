import { describe, expect, test } from "bun:test";
import { appEventRule } from "../../../../lib/device-management/model/app-plan";
import type { IEvent } from "../../../../lib/schema/flow/event";
import { eventInput } from "./use-app";

const bytes = (config: unknown) => [
	...new TextEncoder().encode(JSON.stringify(config)),
];

function record(
	event_type: string,
	config: unknown,
	extra: Partial<IEvent> = {},
): IEvent {
	return {
		id: `evt_${event_type}`,
		name: `A ${event_type}`,
		active: true,
		board_id: "board",
		board_version: [1, 0, 0],
		event_type,
		event_version: [1, 0, 0],
		config: config === undefined ? [] : bytes(config),
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		description: "",
		node_id: "node",
		priority: 0,
		variables: {},
		...extra,
	};
}

describe("what App › Devices reads of an event record (design R2 §6.2)", () => {
	test("the rule reads the route from the record's config; whether it has its own token is a fact, never the token", () => {
		const own = eventInput(
			record("api", {
				sink_type: "http",
				method: "GET",
				path: "/orders",
				auth_token: "secret-of-the-endpoint",
			}),
			null,
		);
		expect(own.ownToken).toBe(true);
		expect(appEventRule(own)).toMatchObject({
			eligible: true,
			kind: "served",
			route: { method: "GET", path: "/orders" },
		});
		expect(
			eventInput(
				record("api", { method: "GET", path: "/o", auth_token: " " }),
				null,
			).ownToken,
		).toBe(false);
		// A config that can't be read says nothing about a token.
		expect(eventInput(record("api", undefined), null)).not.toHaveProperty(
			"ownToken",
		);
		expect(eventInput(record("simple_chat", {}), null)).not.toHaveProperty(
			"ownToken",
		);
	});

	test("a bot's settings and a one-time schedule come from the same config", () => {
		const bot = eventInput(
			record("telegram", {
				bot_token: "123456789:AAHfixture-token-0123456789abcdef",
				chat_whitelist: [],
			}),
			null,
		);
		expect(appEventRule(bot).bot).toMatchObject({
			provider: "telegram",
			open: true,
			savedToken: true,
		});
		const once = eventInput(
			record("cron", {
				scheduled_for: { date: "2026-10-15", time: "09:00" },
				timezone: "Europe/Berlin",
			}),
			null,
		);
		expect(appEventRule(once).once).toMatchObject({
			date: "2026-10-15",
			time: "09:00",
			at: 1792047600,
		});
	});

	test("a form counts its fields and the ones that take a file", () => {
		const input = (name: string, data_type: string) => ({
			id: name,
			name,
			friendly_name: name,
			description: "",
			data_type,
			value_type: "Normal",
			index: 0,
		});
		expect(
			eventInput(
				record(
					"generic_form",
					{},
					{
						inputs: [
							input("title", "String"),
							input("photo", "PathBuf"),
							input("raw", "Byte"),
						],
					},
				),
				null,
			).form,
		).toEqual({ fields: 3, fileFields: 2 });
		expect(eventInput(record("quick_action", {}), null).form).toEqual({
			fields: 0,
			fileFields: 0,
		});
		expect(eventInput(record("http", {}), null)).not.toHaveProperty("form");
	});
});
