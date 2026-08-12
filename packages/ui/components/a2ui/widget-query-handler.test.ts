import { describe, expect, it } from "bun:test";
import { parseWidgetQueryMessage } from "./widget-query-handler";

describe("parseWidgetQueryMessage", () => {
	it("parses the snake_case wire form", () => {
		const parsed = parseWidgetQueryMessage({
			type: "widgetQuery",
			request_id: "req-1",
			instance_id: "inst-1",
			query: "getSelection",
			args: { limit: 5 },
			timeout_ms: 5000,
		});
		expect(parsed).toEqual({
			requestId: "req-1",
			instanceId: "inst-1",
			query: "getSelection",
			args: { limit: 5 },
			timeoutMs: 5000,
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
