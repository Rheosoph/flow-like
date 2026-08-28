import type { IChannelClientDescriptor, IChannelPush } from "../schema/channel";
import {
	type ChannelPushOptions,
	errorMessage,
	readBodyExcerpt,
	timeoutSignal,
} from "./util";

export const HTTP_PUSH_TIMEOUT_MS = 30_000;

export type HttpChannelDescriptor = Extract<
	IChannelClientDescriptor,
	{ type: "http" }
>;

export async function pushHttp(
	descriptor: HttpChannelDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions = {},
): Promise<void> {
	const timeout = timeoutSignal(HTTP_PUSH_TIMEOUT_MS, options.signal);
	try {
		let response: Response;
		try {
			response = await fetch(descriptor.push_url, {
				method: "POST",
				headers: {
					"content-type": "application/json",
					authorization: `Bearer ${descriptor.token}`,
				},
				body: JSON.stringify(push),
				signal: timeout.signal,
			});
		} catch (error) {
			if (timeout.timedOut()) {
				throw new Error(
					`Channel push to '${descriptor.push_url}' timed out after ${HTTP_PUSH_TIMEOUT_MS} ms.`,
				);
			}
			if (timeout.signal.aborted) {
				throw new Error(
					`Channel push to '${descriptor.push_url}' was aborted.`,
				);
			}
			throw new Error(
				`Channel push to '${descriptor.push_url}' failed: ${errorMessage(error)}`,
			);
		}
		if (!response.ok) {
			const excerpt = await readBodyExcerpt(response);
			throw new Error(
				`Channel push to '${descriptor.push_url}' failed (${response.status}): ${excerpt || response.statusText}`,
			);
		}
	} finally {
		timeout.dispose();
	}
}
