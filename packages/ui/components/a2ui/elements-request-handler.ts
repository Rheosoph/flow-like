import { isChannelHandle, replyToChannel } from "../../lib/channel";
import { errorMessage } from "../../lib/channel/util";
import type { IChannelHandle } from "../../lib/schema/channel";
import {
	type ElementSource,
	materializeSurfaceElements,
} from "./element-materializer";

const DEFAULT_ELEMENTS_REQUEST_TIMEOUT_MS = 10_000;
const NO_LIVE_SURFACE = "no live surface for this run";

/**
 * Wire shape of the `requestElements` a2ui message (run → frontend live read
 * of page elements). Fields arrive snake_case from serde; camelCase accepted
 * for normalized paths.
 */
export interface ElementsRequest {
	requestId: string;
	selectors: string[];
	timeoutMs: number;
	/** Where the answer goes; a request without one cannot be answered. */
	channel: IChannelHandle | null;
}

export type ElementsRequestReply =
	| { ok: true; elements: Record<string, unknown> }
	| { ok: false; error: string };

export interface ElementsRequestHandlerOptions {
	materialize?: typeof materializeSurfaceElements;
	reply?: typeof replyToChannel;
}

function isElementsRequestMessage(
	message: unknown,
): message is Record<string, unknown> {
	return (
		typeof message === "object" &&
		message !== null &&
		(message as Record<string, unknown>).type === "requestElements"
	);
}

function stringList(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	return value.filter((entry): entry is string => typeof entry === "string");
}

/**
 * `null` for other message types and for the legacy `{ element_ids }` shape,
 * which carries no request id and therefore nothing to answer.
 */
export function parseElementsRequestMessage(
	message: unknown,
): ElementsRequest | null {
	if (!isElementsRequestMessage(message)) return null;

	const requestId = message.request_id ?? message.requestId;
	if (typeof requestId !== "string" || requestId.length === 0) return null;

	const timeoutRaw = message.timeout_ms ?? message.timeoutMs;
	const timeoutMs =
		typeof timeoutRaw === "number" && timeoutRaw > 0
			? timeoutRaw
			: DEFAULT_ELEMENTS_REQUEST_TIMEOUT_MS;

	return {
		requestId,
		selectors: stringList(message.selectors),
		timeoutMs,
		channel: isChannelHandle(message.channel) ? message.channel : null,
	};
}

/** Channel transports cap a push well below this; a bigger answer would only time out. */
const MAX_REPLY_BYTES = 512 * 1024;

const inFlight = new Set<string>();

/**
 * Intercept a streamed a2ui message. Returns true when the message is a
 * `requestElements` request (callers should skip further processing); the
 * request is answered asynchronously from `source()`, the live surface this
 * client owns for the run, or with an error when there is none.
 */
export function handleElementsRequestMessage(
	message: unknown,
	source: () => ElementSource | null,
	options: ElementsRequestHandlerOptions = {},
): boolean {
	if (!isElementsRequestMessage(message)) return false;
	const request = parseElementsRequestMessage(message);
	if (!request || inFlight.has(request.requestId)) return true;

	const channel = request.channel;
	if (!channel) {
		console.warn(
			"[a2ui] requestElements cannot be answered: the message carries no channel",
			request.requestId,
		);
		return true;
	}
	inFlight.add(request.requestId);
	const materialize = options.materialize ?? materializeSurfaceElements;
	const reply = options.reply ?? replyToChannel;

	void (async () => {
		let response: ElementsRequestReply;
		try {
			const live = source();
			if (live) {
				const elements = materialize(live, request.selectors, live.widgetScope);
				const bytes = JSON.stringify(elements).length;
				response =
					bytes > MAX_REPLY_BYTES
						? {
								ok: false,
								error: `answer too large (${bytes} bytes, limit ${MAX_REPLY_BYTES}) — narrow the selectors`,
							}
						: { ok: true, elements };
			} else {
				response = { ok: false, error: NO_LIVE_SURFACE };
			}
		} catch (error) {
			response = { ok: false, error: errorMessage(error) };
		}
		try {
			await reply(channel, response);
		} catch (error) {
			console.warn("[a2ui] failed to deliver requestElements response", error);
		} finally {
			inFlight.delete(request.requestId);
		}
	})();

	return true;
}
