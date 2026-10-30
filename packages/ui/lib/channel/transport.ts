// Client half of Channels: deliver a reply / cancel / steer to the waiter behind a run through the
// transport its handle names, falling back to the API push endpoint when the transport fails.

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
 * Deliver one push through `handle.transport`; on any transport error retry once through
 * `handle.fallback` (the API push endpoint) when the handle carries one.
 */
export async function rawPushChannel(
	handle: IChannelHandle,
	push: Omit<IChannelPush, "channel_id">,
	options: ChannelPushOptions = {},
): Promise<void> {
	const body: IChannelPush = { ...push, channel_id: handle.channel_id };
	const deliveryOptions =
		handle.transport.type === "nats"
			? { ...options, deliveryId: options.deliveryId ?? crypto.randomUUID() }
			: options;
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
