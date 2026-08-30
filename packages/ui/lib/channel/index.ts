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
import { pushHttp } from "./http";
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
		case "aws_mqtt":
			return pushAwsMqtt(descriptor, push, options);
		case "azure_web_pubsub":
			return pushAzureWebPubSub(descriptor, push, options);
		case "gcp_firebase_rtdb":
			return pushFirebaseRtdb(descriptor, push, options);
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
export async function pushToChannel(
	handle: IChannelHandle,
	push: Omit<IChannelPush, "channel_id">,
	options: ChannelPushOptions = {},
): Promise<void> {
	const body: IChannelPush = { ...push, channel_id: handle.channel_id };
	try {
		await deliver(handle.transport, body, options);
	} catch (primary) {
		if (!handle.fallback || options.signal?.aborted) throw primary;
		console.warn(
			`[channel] ${handle.transport.type} push for channel '${handle.channel_id}' failed, retrying through ${handle.fallback.type}: ${errorMessage(primary)}`,
		);
		try {
			await deliver(handle.fallback, body, options);
		} catch (fallback) {
			throw new Error(
				`${errorMessage(fallback)} (after ${handle.transport.type} failed: ${errorMessage(primary)})`,
				{ cause: fallback },
			);
		}
	}
}

/** Answer the request the handle was issued for. */
export function replyToChannel(
	handle: IChannelHandle,
	value: unknown,
	options?: ChannelPushOptions,
): Promise<void> {
	if (!handle.request_id) {
		return Promise.reject(
			new Error(
				`Channel handle for '${handle.channel_id}' carries no request_id; nothing to reply to.`,
			),
		);
	}
	return pushToChannel(
		handle,
		{ request_id: handle.request_id, kind: "reply", value },
		options,
	);
}

/** Stop the run behind the channel; idempotent on the waiter side. */
export function cancelChannel(
	handle: IChannelHandle,
	options?: ChannelPushOptions,
): Promise<void> {
	return pushToChannel(handle, { kind: "cancel", value: null }, options);
}

/** Push steering text the waiter drains at its next boundary. */
export function steerChannel(
	handle: IChannelHandle,
	text: string,
	options?: ChannelPushOptions,
): Promise<void> {
	return pushToChannel(handle, { kind: "inbound", value: text }, options);
}

/** Drop cached transport sessions (Azure sockets, Firebase id tokens). Intended for tests. */
export function resetChannelClientCaches(): void {
	resetAzureWebPubSubConnections();
	resetFirebaseSessions();
}
