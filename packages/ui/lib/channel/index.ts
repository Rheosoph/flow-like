import { getExecutionClientForChannel } from "../execution/runtime-client";
import type { IChannelHandle, IChannelPush } from "../schema/channel";
import { rawPushChannel } from "./transport";
import type { ChannelPushOptions } from "./util";

export { isChannelHandle } from "./handle";
export { resetChannelClientCaches } from "./transport";
export type { ChannelPushOptions } from "./util";

/** The shared session arbitrates replies before any cloud or HTTP transport sends them. */
export async function pushToChannel(
	handle: IChannelHandle,
	push: Omit<IChannelPush, "channel_id">,
	options: ChannelPushOptions = {},
): Promise<void> {
	const owner = getExecutionClientForChannel(handle);
	if (owner && (push.kind ?? "reply") === "reply") {
		if (options.signal?.aborted)
			throw options.signal.reason ?? new Error("Reply cancelled");
		if (push.request_id !== handle.request_id) {
			throw new Error("The reply does not belong to this channel request.");
		}
		await owner.submitReply(handle, push.value);
		return;
	}
	await rawPushChannel(handle, push, options);
}

/** Claim a frontend action before performing its side effects in this tab. */
export async function claimChannelRequest(
	handle: IChannelHandle,
): Promise<boolean> {
	const owner = getExecutionClientForChannel(handle);
	return owner ? owner.claimRequest(handle) : true;
}

export function isSharedChannel(handle: IChannelHandle): boolean {
	return !!getExecutionClientForChannel(handle);
}

export function replyToChannel(
	handle: IChannelHandle,
	value: unknown,
	options?: ChannelPushOptions,
): Promise<void> {
	if (!handle.request_id) {
		return Promise.reject(
			new Error(`Channel '${handle.channel_id}' has no request_id to answer.`),
		);
	}
	return pushToChannel(
		handle,
		{ request_id: handle.request_id, kind: "reply", value },
		options,
	);
}

export function cancelChannel(
	handle: IChannelHandle,
	options?: ChannelPushOptions,
): Promise<void> {
	return pushToChannel(handle, { kind: "cancel", value: null }, options);
}

export function steerChannel(
	handle: IChannelHandle,
	text: string,
	options?: ChannelPushOptions,
): Promise<void> {
	return pushToChannel(handle, { kind: "inbound", value: text }, options);
}
