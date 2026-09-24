import { afterEach, describe, expect, it, spyOn } from "bun:test";
import {
	MAX_HOME_LAYOUT_BYTES,
	homeLayoutByteLength,
} from "../../../components/home/home-layout";
import { parseHomeLayoutJson } from "../../../components/home/home-layout-json";
import type { IHomeLayout } from "../../../components/home/types";
import type { IChannelHandle, IChannelPush } from "../../schema/channel";
import {
	AWS_MAX_REPLY_BYTES,
	AWS_MQTT_PAYLOAD_BYTES,
	awsReplyPayloads,
} from "../aws-reply-chunks";
import { replyToChannel } from "../index";

const byteLength = (text: string) => new TextEncoder().encode(text).length;
const reply = (value: unknown): IChannelPush => ({
	channel_id: "run-1",
	request_id: "req-1",
	kind: "reply",
	value,
});

function reassemble(payloads: string[]): IChannelPush {
	if (payloads.length === 1 && JSON.parse(payloads[0]).kind === "reply") {
		return JSON.parse(payloads[0]);
	}
	return JSON.parse(
		Buffer.concat(
			payloads.map((payload, index) => {
				expect(byteLength(payload)).toBeLessThanOrEqual(AWS_MQTT_PAYLOAD_BYTES);
				const frame = JSON.parse(payload);
				expect(frame).toMatchObject({
					kind: "reply_chunk",
					index,
					total: payloads.length,
					channel_id: "run-1",
					request_id: "req-1",
				});
				return Buffer.from(frame.data, "base64");
			}),
		).toString("utf8"),
	);
}

function maximumHomeLayout(title: string): IHomeLayout {
	const layout: IHomeLayout = {
		version: 1,
		title,
		widgets: [
			{
				id: "information",
				type: "information",
				size: { columns: 12, rows: 3 },
				appearance: { variant: "card", accent: "neutral" },
				config: { body: "" },
			},
		],
	};
	layout.widgets[0].config.body = "x".repeat(
		MAX_HOME_LAYOUT_BYTES - homeLayoutByteLength(layout),
	);
	expect(homeLayoutByteLength(layout)).toBe(MAX_HOME_LAYOUT_BYTES);
	expect(parseHomeLayoutJson(JSON.stringify(layout)).ok).toBe(true);
	return layout;
}

describe("AWS complete-message budgets", () => {
	it("sends an exact-limit envelope unchanged and chunks one byte above it", () => {
		const overhead = byteLength(JSON.stringify(reply("")));
		const exact = reply("x".repeat(AWS_MQTT_PAYLOAD_BYTES - overhead));
		expect(awsReplyPayloads(exact)).toEqual([JSON.stringify(exact)]);
		const above = reply(`${exact.value}x`);
		expect(awsReplyPayloads(above)).toHaveLength(3);
		expect(reassemble(awsReplyPayloads(above))).toEqual(above);
	});

	it("preserves a maximum valid Home layout and three full comparison layouts", () => {
		const value = {
			status: "ok",
			layout: maximumHomeLayout("Current"),
			base_layout: maximumHomeLayout("Base"),
			default_layout: maximumHomeLayout("Default"),
			comparison_layout: maximumHomeLayout("Comparison"),
		};
		const push = reply({ approved: true, result: value });
		expect(reassemble(awsReplyPayloads(push))).toEqual(push);
		expect(
			reassemble(
				awsReplyPayloads(reply({ canonical_layout: value.layout, issues: [] })),
			),
		).toEqual(reply({ canonical_layout: value.layout, issues: [] }));
	});

	it("measures UTF-8 and preserves characters split between byte chunks", () => {
		const push = reply({ body: '😊漢字\\"\n'.repeat(20_000) });
		expect(reassemble(awsReplyPayloads(push))).toEqual(push);
	});

	it("rejects oversized replies and oversized identifiers with bounded errors", () => {
		expect(() =>
			awsReplyPayloads(reply("x".repeat(AWS_MAX_REPLY_BYTES))),
		).toThrow("2 MiB");
		expect(() =>
			awsReplyPayloads({
				...reply("x".repeat(AWS_MQTT_PAYLOAD_BYTES)),
				channel_id: "a".repeat(50_000),
			}),
		).toThrow("chunk envelope");
		expect(() =>
			awsReplyPayloads({
				...reply("x".repeat(AWS_MQTT_PAYLOAD_BYTES)),
				kind: "inbound",
			}),
		).toThrow("must be replies");
		expect(() =>
			awsReplyPayloads({
				...reply("x".repeat(AWS_MQTT_PAYLOAD_BYTES)),
				request_id: null,
			}),
		).toThrow("request_id");
	});
});

let restore: (() => void) | undefined;
afterEach(() => {
	restore?.();
	restore = undefined;
});

describe("AWS delivery and HTTP fallback", () => {
	const fallbackUrl = "https://api.example/push";
	const handle: IChannelHandle = {
		channel_id: "run-1",
		request_id: "req-1",
		expires_at: 4_102_444_800,
		transport: {
			type: "aws_mqtt",
			endpoint: "data.iot.example",
			region: "eu-central-1",
			target_client_id: "run-1",
			topic: "runs/run-1/inbox",
			credentials: {
				access_key_id: "AKID",
				secret_access_key: "secret",
				session_token: "token",
				expiration: 4_102_444_800,
			},
		},
		fallback: {
			type: "http",
			push_url: fallbackUrl,
			token: "token",
		},
	};

	it("publishes bounded frames directly and preserves the original reply on fallback", async () => {
		const original = globalThis.fetch;
		const calls: { url: string; body: string }[] = [];
		const warn = spyOn(console, "warn").mockImplementation(() => undefined);
		restore = () => {
			globalThis.fetch = original;
			warn.mockRestore();
		};
		globalThis.fetch = (async (url, init) => {
			calls.push({ url: String(url), body: String(init?.body) });
			return new Response(null, { status: calls.length === 2 ? 503 : 204 });
		}) as typeof fetch;
		const value = { canonical_layout: maximumHomeLayout("Current") };
		await replyToChannel(handle, value);
		expect(calls).toHaveLength(3);
		for (const call of calls.slice(0, 2)) {
			expect(byteLength(call.body)).toBeLessThanOrEqual(AWS_MQTT_PAYLOAD_BYTES);
			expect(JSON.parse(call.body).kind).toBe("reply_chunk");
		}
		expect(calls[2].url).toBe(fallbackUrl);
		expect(JSON.parse(calls[2].body)).toEqual(reply(value));
	});
});
