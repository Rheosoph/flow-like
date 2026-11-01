// Remote replies use the API push capability. The API forwards to the execution's
// scoped transport; old handles without an API capability retain their original path.

import type {
	IChannelClientDescriptor,
	IChannelHandle,
	IChannelPush,
} from "../schema/channel";
import { pushAwsMqtt } from "./aws-mqtt";
import {
	pushAzureWebPubSub,
	resetAzureWebPubSubConnections,
} from "./azure-web-pubsub";
import { pushFirebaseRtdb, resetFirebaseSessions } from "./gcp-firebase-rtdb";
import { isScopedHttpChannel, pushHttp } from "./http";
import { pushInProcess } from "./in-process";
import { pushNats, resetNatsConnections } from "./nats";
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
		case "aws_mqtt":
			return pushAwsMqtt(descriptor, push, options);
		case "azure_web_pubsub":
			return pushAzureWebPubSub(descriptor, push, options);
		case "gcp_firebase_rtdb":
			return pushFirebaseRtdb(descriptor, push, options);
		case "nats":
			return pushNats(descriptor, push, options);
		default:
			return Promise.reject(
				new Error(
					`Unknown channel transport '${(descriptor as { type?: unknown }).type}' for channel '${push.channel_id}'.`,
				),
			);
	}
}

/**
 * Prefer the API capability on legacy cloud handles, so replying needs no additional
 * frontend provider connection. New remote handles already name this HTTP endpoint.
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
		// A failed API push remains retryable through the same authority. Opening a
		// provider write connection here would bypass the shared-connection design.
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

/** Drop cached transport sessions when the execution runtime retires. */
export function resetChannelClientCaches(): void {
	resetAzureWebPubSubConnections();
	resetFirebaseSessions();
	resetNatsConnections();
}
