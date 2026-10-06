import { describe, expect, it } from "bun:test";
import { registerScopedHttpChannel } from "../../lib/channel/http";
import type { IChannelPush } from "../../lib/schema/channel";
import { registerMicroWidgetBridge } from "./micro-widget-host";
import {
	handleWidgetQueryMessage,
	parseWidgetQueryMessage,
} from "./widget-query-handler";

const channel = {
	channel_id: "run-1",
	request_id: "req-1",
	expires_at: 4_102_444_800,
	transport: { type: "in_process" as const },
};

describe("parseWidgetQueryMessage", () => {
	it("refuses deployed queries before accessing a host widget and returns a scoped error", async () => {
		let hostQueries = 0;
		const unregister = registerMicroWidgetBridge("host-widget", {
			query: async () => {
				hostQueries++;
				return "private Studio state";
			},
		});
		let receive!: (push: IChannelPush) => void;
		const reply = new Promise<IChannelPush>((resolve) => {
			receive = resolve;
		});
		const scoped = registerScopedHttpChannel(async (_descriptor, push) => {
			receive(push);
		});
		try {
			expect(
				handleWidgetQueryMessage({
					type: "widgetQuery",
					request_id: "remote-request",
					instance_id: "host-widget",
					query: "getSelection",
					channel: {
						...channel,
						request_id: "remote-request",
						transport: { type: "http", push_url: scoped.url, token: "token" },
					},
				}),
			).toBe(true);
			expect(await reply).toEqual({
				channel_id: "run-1",
				request_id: "remote-request",
				kind: "reply",
				value: {
					ok: false,
					error:
						"Package widget queries are not available in deployed app sessions.",
				},
			});
			expect(hostQueries).toBe(0);
		} finally {
			unregister();
			scoped.close();
		}
	});

	it("parses the snake_case wire form", () => {
		const parsed = parseWidgetQueryMessage({
			type: "widgetQuery",
			request_id: "req-1",
			instance_id: "inst-1",
			query: "getSelection",
			args: { limit: 5 },
			timeout_ms: 5000,
			channel,
		});
		expect(parsed).toEqual({
			requestId: "req-1",
			instanceId: "inst-1",
			query: "getSelection",
			args: { limit: 5 },
			timeoutMs: 5000,
			channel,
		});
	});

	it("parses the normalized camelCase form", () => {
		const parsed = parseWidgetQueryMessage({
			type: "widgetQuery",
			requestId: "req-2",
			instanceId: "inst-2",
			query: "getValue",
			timeoutMs: 250,
		});
		expect(parsed?.requestId).toBe("req-2");
		expect(parsed?.instanceId).toBe("inst-2");
		expect(parsed?.args).toBeNull();
		expect(parsed?.timeoutMs).toBe(250);
		expect(parsed?.channel).toBeNull();
	});

	it("defaults the timeout when missing or invalid", () => {
		const parsed = parseWidgetQueryMessage({
			type: "widgetQuery",
			request_id: "req-3",
			instance_id: "inst-3",
			query: "getValue",
			timeout_ms: -1,
		});
		expect(parsed?.timeoutMs).toBe(10_000);
	});

	it("drops a malformed channel instead of the whole request", () => {
		const parsed = parseWidgetQueryMessage({
			type: "widgetQuery",
			request_id: "req-4",
			instance_id: "inst-4",
			query: "getValue",
			channel: { channel_id: "run-1" },
		});
		expect(parsed?.requestId).toBe("req-4");
		expect(parsed?.channel).toBeNull();
	});

	it("rejects other message types and malformed requests", () => {
		expect(parseWidgetQueryMessage(null)).toBeNull();
		expect(parseWidgetQueryMessage("widgetQuery")).toBeNull();
		expect(parseWidgetQueryMessage({ type: "upsertElement" })).toBeNull();
		expect(
			parseWidgetQueryMessage({ type: "widgetQuery", request_id: "x" }),
		).toBeNull();
		expect(
			parseWidgetQueryMessage({
				type: "widgetQuery",
				request_id: "x",
				instance_id: 42,
				query: "getValue",
			}),
		).toBeNull();
	});
});
