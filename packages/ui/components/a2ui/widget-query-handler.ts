import { isChannelHandle, replyToChannel } from "../../lib/channel";
import type { IChannelHandle } from "../../lib/schema/channel";
import {
	MICRO_WIDGET_QUERY_TIMEOUT_MS,
	microWidgetHasInstance,
	microWidgetQuery,
} from "./micro-widget-host";

/**
 * Wire shape of the `widgetQuery` a2ui message (run → frontend live request).
 * Fields arrive snake_case from serde; camelCase accepted for normalized paths.
 */
export interface WidgetQueryRequest {
	requestId: string;
	instanceId: string;
	query: string;
	args: unknown;
	timeoutMs: number;
	/** Where the answer goes; a query without one cannot be answered. */
	channel: IChannelHandle | null;
}

export function parseWidgetQueryMessage(
	message: unknown,
): WidgetQueryRequest | null {
	if (typeof message !== "object" || message === null) return null;
	const record = message as Record<string, unknown>;
	if (record.type !== "widgetQuery") return null;

	const requestId = record.request_id ?? record.requestId;
	const instanceId = record.instance_id ?? record.instanceId;
	const query = record.query;
	if (
		typeof requestId !== "string" ||
		typeof instanceId !== "string" ||
		typeof query !== "string"
	) {
		return null;
	}

	const timeoutRaw = record.timeout_ms ?? record.timeoutMs;
	const timeoutMs =
		typeof timeoutRaw === "number" && timeoutRaw > 0
			? timeoutRaw
			: MICRO_WIDGET_QUERY_TIMEOUT_MS;

	return {
		requestId,
		instanceId,
		query,
		args: record.args ?? null,
		timeoutMs,
		channel: isChannelHandle(record.channel) ? record.channel : null,
	};
}

export interface WidgetQueryResponse {
	ok: boolean;
	value?: unknown;
	error?: string;
}

const inFlight = new Set<string>();

/**
 * Intercept a streamed a2ui message. Returns true when the message is a
 * `widgetQuery` request (callers should skip further processing); the query
 * is answered asynchronously iff this surface hosts the live instance —
 * other surfaces stay silent so exactly the hosting one responds.
 */
export function handleWidgetQueryMessage(message: unknown): boolean {
	const request = parseWidgetQueryMessage(message);
	if (!request) return false;

	if (
		!microWidgetHasInstance(request.instanceId) ||
		inFlight.has(request.requestId)
	) {
		return true;
	}
	const channel = request.channel;
	if (!channel) {
		console.warn(
			"[a2ui] widgetQuery cannot be answered: the message carries no channel",
			request.requestId,
		);
		return true;
	}
	inFlight.add(request.requestId);

	void (async () => {
		let response: WidgetQueryResponse;
		try {
			const value = await microWidgetQuery(
				request.instanceId,
				request.query,
				request.args,
				request.timeoutMs,
			);
			response = { ok: true, value };
		} catch (error) {
			response = {
				ok: false,
				error: error instanceof Error ? error.message : String(error),
			};
		}
		try {
			await replyToChannel(channel, response);
		} catch (error) {
			console.warn("[a2ui] failed to deliver widgetQuery response", error);
		} finally {
			inFlight.delete(request.requestId);
		}
	})();

	return true;
}
