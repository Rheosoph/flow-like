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

const SCOPED_CHANNEL_PREFIX = "flow-like-service-channel:";
type ScopedPush = (
	descriptor: HttpChannelDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions,
) => Promise<void>;
const scopedPushes = new Map<string, ScopedPush>();

export function isScopedHttpChannel(
	descriptor: IChannelClientDescriptor,
): boolean {
	return (
		descriptor.type === "http" &&
		descriptor.push_url.startsWith(SCOPED_CHANNEL_PREFIX)
	);
}

/** Registers a session-owned endpoint. Unknown or expired endpoints never use browser fetch. */
export function registerScopedHttpChannel(push: ScopedPush): {
	url: string;
	close(): void;
} {
	const url = `${SCOPED_CHANNEL_PREFIX}${crypto.randomUUID()}`;
	scopedPushes.set(url, push);
	return {
		url,
		close: () => {
			scopedPushes.delete(url);
		},
	};
}

export async function pushHttp(
	descriptor: HttpChannelDescriptor,
	push: IChannelPush,
	options: ChannelPushOptions = {},
): Promise<void> {
	if (descriptor.push_url.startsWith(SCOPED_CHANNEL_PREFIX)) {
		const deliver = scopedPushes.get(descriptor.push_url);
		if (!deliver) throw new Error("The deployed service session has ended.");
		return deliver(descriptor, push, options);
	}
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
