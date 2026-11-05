// Remote replies use the API push capability. The API forwards to the execution's
// scoped transport, so a client never holds provider write credentials.

import type {
	IChannelClientDescriptor,
	IChannelHandle,
	IChannelPush,
} from "../schema/channel";
import { isScopedHttpChannel, pushHttp } from "./http";
import { pushInProcess } from "./in-process";
import { type ChannelPushOptions, errorMessage } from "./util";

export { isChannelHandle } from "./handle";
export type { ChannelPushOptions } from "./util";

function deliver(
	descriptor: IChannelClientDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions,
): Promise<void> {
	switch (descriptor.type) {
		case "http":
			return pushHttp(descriptor, push, options);
		case "in_process":
			return pushInProcess(push);
		default:
			return Promise.reject(
				new Error(
					`Channel '${push.channel_id}' names the '${(descriptor as { type?: unknown }).type}' transport without an API push capability.`,
				),
			);
	}
}

/**
 * Provider handles from older APIs carry the API capability as their fallback; it is the
 * only path a reply takes, so replying needs no frontend provider connection.
 */
export async function rawPushChannel(
	handle: IChannelHandle,
	push: Omit<IChannelPush, "channel_id">,
	options: ChannelPushOptions = {},
): Promise<void> {
	if (options.signal?.aborted)
		throw options.signal.reason ?? new Error("Channel push was aborted.");
	const body: IChannelPush = { ...push, channel_id: handle.channel_id };
	const deliveryOptions =
		handle.transport.type !== "in_process" &&
		!isScopedHttpChannel(handle.transport)
			? { ...options, deliveryId: options.deliveryId ?? crypto.randomUUID() }
			: options;
	if (
		handle.transport.type !== "http" &&
		handle.transport.type !== "in_process" &&
		handle.fallback?.type === "http"
	) {
		// A failed API push remains retryable through the same authority.
		return pushHttp(handle.fallback, body, deliveryOptions);
	}
	try {
		await deliver(handle.transport, body, deliveryOptions);
	} catch (primary) {
		if (
			!handle.fallback ||
			options.signal?.aborted ||
			isScopedHttpChannel(handle.transport)
		)
			throw primary;
		console.warn(
			`[channel] ${handle.transport.type} push for channel '${handle.channel_id}' failed, retrying through ${handle.fallback.type}: ${errorMessage(primary)}`,
		);
		try {
			await deliver(handle.fallback, body, deliveryOptions);
		} catch (fallback) {
			throw new Error(
				`${errorMessage(fallback)} (after ${handle.transport.type} failed: ${errorMessage(primary)})`,
				{ cause: fallback },
			);
		}
	}
}
